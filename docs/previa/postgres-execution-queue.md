# Postgres execution queue

Previa requires Postgres for all operational state. `previa-main` persists an
execution and its jobs before returning an execution ID. Runners claim eligible
jobs directly from Postgres with `FOR UPDATE SKIP LOCKED`, renew fenced leases,
append idempotent events, and publish terminal results. There is no HTTP
execution transport between main and runners.

SQLite remains supported only as the portable project import/export format:

```bash
previa local export --all --output ./previa-projects.sqlite3
previa local import ./previa-projects.sqlite3
```

## Required connections

- `DATABASE_URL` is the owner connection used by `previa-main` for migrations,
  API state, queue projection, maintenance, and retention.
- `PREVIA_QUEUE_DATABASE_URL` is the restricted runner connection. It should
  use the `previa_runner_queue` login, which receives only the queue function
  privileges created by the migrations.

Both variables accept only `postgres://` or `postgresql://` URLs. Never inject
the main owner URL into a runner pod.

`previa up` provisions `postgres:17`, persistent data, a health check, and
distinct generated credentials in the context's mode-0600 `main.env`.
Production Helm installs use existing Secrets instead of provisioning a
database.

## Queue states

External execution state is one of `queued`, `leased`, `running`, `retrying`,
`cancel_requested`, `completed`, `failed`, or `cancelled`. Claims carry a
fencing token, so a runner that loses its lease cannot publish stale results.
`LISTEN/NOTIFY` reduces latency; bounded polling remains the recovery path.

Queue diagnostics are available from:

```text
GET /api/v1/queue/diagnostics
```

The response contains queue depth, oldest eligible job age, retry/dead-letter
counts, projection/event backlog, runner heartbeat/capacity totals, and safe
effective timing configuration. Connection strings and credentials are never
returned.

## Configuration defaults

All timing and sizing values are optional environment variables with validated
application defaults:

| Main | Default |
| --- | ---: |
| `PREVIA_QUEUE_RUNNER_STALE_AFTER_MS` | `15000` |
| `PREVIA_QUEUE_JOB_LEASE_MS` | `30000` |
| `PREVIA_QUEUE_JOB_MAX_ATTEMPTS` | `3` |
| `PREVIA_QUEUE_PROJECTION_LEASE_MS` | `30000` |
| `PREVIA_QUEUE_PROJECTION_POLL_INTERVAL_MS` | `1000` |
| `PREVIA_QUEUE_MAINTENANCE_INTERVAL_MS` | `1000` |
| `PREVIA_QUEUE_RETRY_BACKOFF_BASE_MS` | `1000` |
| `PREVIA_QUEUE_RETRY_BACKOFF_MAX_MS` | `30000` |
| `PREVIA_QUEUE_EVENT_RETENTION_HOURS` | `24` |
| `PREVIA_QUEUE_RUNNER_RETENTION_HOURS` | `168` |

| Runner | Default |
| --- | ---: |
| `PREVIA_QUEUE_HEARTBEAT_INTERVAL_MS` | `5000` |
| `PREVIA_QUEUE_LEASE_RENEW_INTERVAL_MS` | `10000` |
| `PREVIA_QUEUE_POLL_INTERVAL_MS` | `1000` |
| `PREVIA_QUEUE_EVENT_FLUSH_INTERVAL_MS` | `250` |
| `PREVIA_QUEUE_EVENT_BATCH_SIZE` | `200` |
| `PREVIA_QUEUE_EVENT_BUFFER_MAX` | `5000` |

Invalid values fail startup rather than silently falling back.

## Upgrade, backup, and rollback

Back up the Postgres database before upgrading. Apply the main migrations
before starting runners with a newer queue protocol. The `queue_protocol`
record is checked at startup by both processes.

This is a breaking transport boundary: old runners that expect
`/api/v1/tests/e2e`, `/api/v1/tests/load`, reservation lifecycle endpoints, or
telemetry acknowledgement endpoints cannot execute jobs for this main. Roll
back main and runners together to the previous release; do not mix protocol
versions. SQLite exports are project transfer artifacts, not operational
database backups.
