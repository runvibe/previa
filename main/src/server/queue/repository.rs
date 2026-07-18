use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct EnqueueExecution {
    pub id: Uuid,
    pub project_id: String,
    pub pipeline_id: Option<String>,
    pub kind: String,
    pub request_json: Value,
    pub created_by: String,
    pub transaction_id: Option<String>,
    pub max_attempts: i32,
    pub jobs: Vec<EnqueueJob>,
}

#[derive(Debug, Clone)]
pub struct EnqueueJob {
    pub id: Uuid,
    pub shard_index: Option<i32>,
    pub pool: String,
    pub requirements_json: Value,
    pub payload_json: Value,
    pub priority: i32,
}

#[derive(Debug, Clone)]
pub struct ProjectionLease {
    pub execution_id: Uuid,
    pub owner: Uuid,
    pub lease_epoch: i64,
    pub last_event_id: i64,
    pub status: String,
    pub snapshot_json: Value,
}

#[derive(Debug, Clone)]
pub struct QueueEventRecord {
    pub id: i64,
    pub execution_id: Uuid,
    pub job_id: Uuid,
    pub runner_id: Uuid,
    pub attempt: i32,
    pub lease_epoch: i64,
    pub seq: i64,
    pub event_type: String,
    pub elapsed_ms: i64,
    pub payload_json: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct QueueRepository {
    pool: PgPool,
}

impl QueueRepository {
    pub async fn connect(database_url: &str, max_connections: u32) -> Result<Self, sqlx::Error> {
        if !(database_url.starts_with("postgres://") || database_url.starts_with("postgresql://")) {
            return Err(sqlx::Error::Configuration(
                "queue database must use Postgres".into(),
            ));
        }
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .connect(database_url)
            .await?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn protocol_version(&self) -> Result<i32, sqlx::Error> {
        sqlx::query_scalar("SELECT protocol_version FROM queue_protocol WHERE id = 1")
            .fetch_one(&self.pool)
            .await
    }

    pub async fn enqueue_execution(&self, request: &EnqueueExecution) -> Result<Uuid, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let shard_count = i32::try_from(request.jobs.len())
            .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
        sqlx::query(
            "INSERT INTO executions (
                id, project_id, pipeline_id, kind, status, desired_status,
                request_json, shard_count, max_attempts, created_by, transaction_id
             ) VALUES ($1, $2, $3, $4, 'queued', 'running', $5, $6, $7, $8, $9)",
        )
        .bind(request.id)
        .bind(&request.project_id)
        .bind(&request.pipeline_id)
        .bind(&request.kind)
        .bind(&request.request_json)
        .bind(shard_count)
        .bind(request.max_attempts)
        .bind(&request.created_by)
        .bind(&request.transaction_id)
        .execute(&mut *tx)
        .await?;

        for job in &request.jobs {
            sqlx::query(
                "INSERT INTO execution_jobs (
                    id, execution_id, kind, shard_index, pool,
                    requirements_json, payload_json, priority, status, max_attempts
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'queued', $9)",
            )
            .bind(job.id)
            .bind(request.id)
            .bind(&request.kind)
            .bind(job.shard_index)
            .bind(&job.pool)
            .bind(&job.requirements_json)
            .bind(&job.payload_json)
            .bind(job.priority)
            .bind(request.max_attempts)
            .execute(&mut *tx)
            .await?;
        }

        sqlx::query(
            "INSERT INTO execution_snapshots (
                execution_id, status, snapshot_json
             ) VALUES ($1, 'queued', $2)",
        )
        .bind(request.id)
        .bind(serde_json::json!({
            "executionId": request.id.to_string(),
            "status": "queued",
            "kind": request.kind,
        }))
        .execute(&mut *tx)
        .await?;
        sqlx::query("SELECT pg_notify('previa_jobs', $1)")
            .bind(request.id.to_string())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(request.id)
    }

    pub async fn cancel_execution(&self, execution_id: Uuid) -> Result<bool, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let changed = sqlx::query(
            "UPDATE executions
             SET desired_status = 'cancelled',
                 status = CASE
                     WHEN status IN ('queued', 'running') THEN 'cancel_requested'
                     ELSE status
                 END,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = $1
               AND status NOT IN ('completed', 'failed', 'cancelled')",
        )
        .bind(execution_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0;
        if changed {
            sqlx::query(
                "UPDATE execution_jobs
                 SET status = 'cancelled',
                     finished_at = CURRENT_TIMESTAMP,
                     updated_at = CURRENT_TIMESTAMP
                 WHERE execution_id = $1
                   AND status IN ('queued', 'retry_wait')",
            )
            .bind(execution_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query("SELECT pg_notify('previa_control', $1)")
                .bind(execution_id.to_string())
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(changed)
    }

    pub async fn claim_projection(
        &self,
        owner: Uuid,
        lease_duration: Duration,
    ) -> Result<Option<ProjectionLease>, sqlx::Error> {
        let lease_ms = i64::try_from(lease_duration.as_millis())
            .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
        let row = sqlx::query(
            "WITH candidate AS (
                SELECT execution_id
                FROM execution_snapshots
                WHERE (
                    projection_lease_expires_at IS NULL
                    OR projection_lease_expires_at <= CURRENT_TIMESTAMP
                    OR projection_owner = $1
                )
                  AND (
                    status NOT IN ('completed', 'failed', 'cancelled')
                    OR EXISTS (
                        SELECT 1 FROM execution_events event
                        WHERE event.execution_id = execution_snapshots.execution_id
                          AND event.id > execution_snapshots.last_event_id
                    )
                  )
                ORDER BY updated_at ASC
                FOR UPDATE SKIP LOCKED
                LIMIT 1
             )
             UPDATE execution_snapshots snapshot
             SET projection_owner = $1,
                 projection_lease_epoch = projection_lease_epoch + 1,
                 projection_lease_expires_at =
                    CURRENT_TIMESTAMP + ($2 * INTERVAL '1 millisecond'),
                 updated_at = CURRENT_TIMESTAMP
             FROM candidate
             WHERE snapshot.execution_id = candidate.execution_id
             RETURNING snapshot.execution_id, snapshot.projection_lease_epoch,
                       snapshot.last_event_id, snapshot.status, snapshot.snapshot_json",
        )
        .bind(owner)
        .bind(lease_ms)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            Ok(ProjectionLease {
                execution_id: row.try_get("execution_id")?,
                owner,
                lease_epoch: row.try_get("projection_lease_epoch")?,
                last_event_id: row.try_get("last_event_id")?,
                status: row.try_get("status")?,
                snapshot_json: row.try_get("snapshot_json")?,
            })
        })
        .transpose()
    }

    pub async fn read_events_after(
        &self,
        execution_id: Uuid,
        last_event_id: i64,
        limit: i64,
    ) -> Result<Vec<QueueEventRecord>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, execution_id, job_id, runner_id, attempt, lease_epoch,
                    seq, event_type, elapsed_ms, payload_json, created_at
             FROM execution_events
             WHERE execution_id = $1 AND id > $2
             ORDER BY id ASC
             LIMIT $3",
        )
        .bind(execution_id)
        .bind(last_event_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(QueueEventRecord {
                    id: row.try_get("id")?,
                    execution_id: row.try_get("execution_id")?,
                    job_id: row.try_get("job_id")?,
                    runner_id: row.try_get("runner_id")?,
                    attempt: row.try_get("attempt")?,
                    lease_epoch: row.try_get("lease_epoch")?,
                    seq: row.try_get("seq")?,
                    event_type: row.try_get("event_type")?,
                    elapsed_ms: row.try_get("elapsed_ms")?,
                    payload_json: row.try_get("payload_json")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn store_snapshot(
        &self,
        lease: &ProjectionLease,
        status: &str,
        snapshot_json: &Value,
        last_event_id: i64,
    ) -> Result<bool, sqlx::Error> {
        Ok(sqlx::query(
            "UPDATE execution_snapshots
             SET version = version + 1,
                 last_event_id = $4,
                 status = $5,
                 snapshot_json = $6,
                 updated_at = CURRENT_TIMESTAMP
             WHERE execution_id = $1
               AND projection_owner = $2
               AND projection_lease_epoch = $3
               AND projection_lease_expires_at > CURRENT_TIMESTAMP
               AND last_event_id <= $4",
        )
        .bind(lease.execution_id)
        .bind(lease.owner)
        .bind(lease.lease_epoch)
        .bind(last_event_id)
        .bind(status)
        .bind(snapshot_json)
        .execute(&self.pool)
        .await?
        .rows_affected()
            > 0)
    }

    pub async fn reap_expired_jobs(
        &self,
        backoff_base: Duration,
        backoff_max: Duration,
    ) -> Result<u64, sqlx::Error> {
        let base_ms = i64::try_from(backoff_base.as_millis())
            .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
        let max_ms = i64::try_from(backoff_max.as_millis())
            .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
        let mut tx = self.pool.begin().await?;
        let expired = sqlx::query(
            "UPDATE execution_jobs
             SET status = CASE
                     WHEN attempt >= max_attempts THEN 'dead_letter'
                     ELSE 'retry_wait'
                 END,
                 available_at = CASE
                     WHEN attempt < max_attempts THEN CURRENT_TIMESTAMP + (
                         LEAST($1 * (2::BIGINT ^ GREATEST(attempt - 1, 0)), $2)
                         * INTERVAL '1 millisecond'
                     )
                     ELSE available_at
                 END,
                 finished_at = CASE
                     WHEN attempt >= max_attempts THEN CURRENT_TIMESTAMP
                     ELSE NULL
                 END,
                 last_error = 'job lease expired',
                 runner_id = CASE WHEN attempt < max_attempts THEN NULL ELSE runner_id END,
                 lease_token = NULL,
                 lease_expires_at = NULL,
                 updated_at = CURRENT_TIMESTAMP
             WHERE status IN ('leased', 'running')
               AND lease_expires_at <= CURRENT_TIMESTAMP",
        )
        .bind(base_ms)
        .bind(max_ms)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        let ready = sqlx::query(
            "UPDATE execution_jobs
             SET status = 'queued', updated_at = CURRENT_TIMESTAMP
             WHERE status = 'retry_wait' AND available_at <= CURRENT_TIMESTAMP",
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if expired + ready > 0 {
            sqlx::query("SELECT pg_notify('previa_jobs', 'reaper')")
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(expired + ready)
    }
}
