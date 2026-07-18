use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use sqlx::postgres::{PgListener, PgPoolOptions};
use sqlx::{PgPool, Row};
use tokio::sync::Mutex;
use uuid::Uuid;

use previa_runner::queue::QUEUE_PROTOCOL_VERSION;

#[derive(Debug, Clone)]
pub struct RunnerRegistration {
    pub name: String,
    pub pool: String,
    pub version: String,
    pub supported_kinds: Vec<String>,
    pub capabilities_json: Value,
    pub labels_json: Value,
    pub max_e2e_slots: i32,
    pub max_load_slots: i32,
    pub heartbeat_interval: Duration,
}

impl RunnerRegistration {
    pub fn from_env() -> Result<Self, String> {
        let supported_kinds = std::env::var("PREVIA_RUNNER_SUPPORTED_KINDS")
            .unwrap_or_else(|_| "e2e,load".to_owned())
            .split(',')
            .map(str::trim)
            .filter(|kind| !kind.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if supported_kinds.is_empty()
            || supported_kinds
                .iter()
                .any(|kind| kind != "e2e" && kind != "load")
        {
            return Err("PREVIA_RUNNER_SUPPORTED_KINDS must contain e2e, load, or both".to_owned());
        }
        Ok(Self {
            name: std::env::var("PREVIA_RUNNER_NAME")
                .unwrap_or_else(|_| format!("runner-{}", Uuid::new_v4())),
            pool: std::env::var("PREVIA_RUNNER_POOL").unwrap_or_else(|_| "default".to_owned()),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            supported_kinds,
            capabilities_json: json_env("PREVIA_RUNNER_CAPABILITIES_JSON")?,
            labels_json: json_env("PREVIA_RUNNER_LABELS_JSON")?,
            max_e2e_slots: positive_i32_env("PREVIA_RUNNER_MAX_E2E_SLOTS", 1)?,
            max_load_slots: positive_i32_env("PREVIA_RUNNER_MAX_LOAD_SLOTS", 1)?,
            heartbeat_interval: Duration::from_millis(5_000),
        })
    }
}

fn json_env(name: &str) -> Result<Value, String> {
    let raw = std::env::var(name).unwrap_or_else(|_| "{}".to_owned());
    let value: Value = serde_json::from_str(&raw).map_err(|error| format!("{name}: {error}"))?;
    if !value.is_object() {
        return Err(format!("{name} must be a JSON object"));
    }
    Ok(value)
}

fn positive_i32_env(name: &str, default: i32) -> Result<i32, String> {
    let value = std::env::var(name)
        .ok()
        .map(|raw| {
            raw.parse::<i32>()
                .map_err(|_| format!("{name} must be an integer"))
        })
        .transpose()?
        .unwrap_or(default);
    if value < 0 {
        return Err(format!("{name} must be zero or greater"));
    }
    Ok(value)
}

#[derive(Debug, Clone)]
pub struct RunnerIdentity {
    pub runner_id: Uuid,
    pub session_token: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClaimedJob {
    pub job_id: Uuid,
    pub execution_id: Uuid,
    pub kind: String,
    pub shard_index: Option<i32>,
    pub payload_json: Value,
    pub attempt: i32,
    pub lease_epoch: i64,
    pub lease_token: Uuid,
    pub lease_expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobFencing {
    pub job_id: Uuid,
    pub attempt: i32,
    pub lease_epoch: i64,
    pub lease_token: Uuid,
}

impl ClaimedJob {
    pub fn fencing(&self) -> JobFencing {
        JobFencing {
            job_id: self.job_id,
            attempt: self.attempt,
            lease_epoch: self.lease_epoch,
            lease_token: self.lease_token,
        }
    }
}

#[derive(Clone)]
pub struct RunnerQueueRepository {
    pool: PgPool,
    listener: Arc<Mutex<PgListener>>,
}

impl RunnerQueueRepository {
    pub async fn connect(database_url: &str, max_connections: u32) -> Result<Self, String> {
        if !(database_url.starts_with("postgres://") || database_url.starts_with("postgresql://")) {
            return Err("queue database must use Postgres".to_owned());
        }
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .connect(database_url)
            .await
            .map_err(safe_database_error)?;
        let mut listener = PgListener::connect_with(&pool)
            .await
            .map_err(safe_database_error)?;
        listener
            .listen_all(["previa_jobs", "previa_control"])
            .await
            .map_err(safe_database_error)?;
        Ok(Self {
            pool,
            listener: Arc::new(Mutex::new(listener)),
        })
    }

    pub async fn register(
        &self,
        registration: &RunnerRegistration,
    ) -> Result<RunnerIdentity, String> {
        let heartbeat_ms = i64::try_from(registration.heartbeat_interval.as_millis())
            .map_err(|error| error.to_string())?;
        let row = sqlx::query(
            "SELECT * FROM queue_register_runner(
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10
             )",
        )
        .bind(&registration.name)
        .bind(&registration.pool)
        .bind(QUEUE_PROTOCOL_VERSION.0)
        .bind(&registration.version)
        .bind(&registration.supported_kinds)
        .bind(&registration.capabilities_json)
        .bind(&registration.labels_json)
        .bind(registration.max_e2e_slots)
        .bind(registration.max_load_slots)
        .bind(heartbeat_ms)
        .fetch_one(&self.pool)
        .await
        .map_err(safe_database_error)?;
        Ok(RunnerIdentity {
            runner_id: row.get("runner_id"),
            session_token: row.get("runner_session_token"),
        })
    }

    pub async fn heartbeat(&self, identity: &RunnerIdentity, status: &str) -> Result<bool, String> {
        sqlx::query_scalar("SELECT queue_heartbeat_runner($1, $2, $3)")
            .bind(identity.runner_id)
            .bind(&identity.session_token)
            .bind(status)
            .fetch_one(&self.pool)
            .await
            .map_err(safe_database_error)
    }

    pub async fn claim_job(
        &self,
        identity: &RunnerIdentity,
        lease: Duration,
    ) -> Result<Option<ClaimedJob>, String> {
        let lease_ms = i64::try_from(lease.as_millis()).map_err(|error| error.to_string())?;
        let row = sqlx::query("SELECT * FROM queue_claim_job($1, $2, $3)")
            .bind(identity.runner_id)
            .bind(&identity.session_token)
            .bind(lease_ms)
            .fetch_optional(&self.pool)
            .await
            .map_err(safe_database_error)?;
        row.map(|row| {
            Ok(ClaimedJob {
                job_id: row.try_get("job_id").map_err(safe_database_error)?,
                execution_id: row.try_get("execution_id").map_err(safe_database_error)?,
                kind: row.try_get("kind").map_err(safe_database_error)?,
                shard_index: row.try_get("shard_index").map_err(safe_database_error)?,
                payload_json: row.try_get("payload_json").map_err(safe_database_error)?,
                attempt: row.try_get("attempt").map_err(safe_database_error)?,
                lease_epoch: row.try_get("lease_epoch").map_err(safe_database_error)?,
                lease_token: row.try_get("lease_token").map_err(safe_database_error)?,
                lease_expires_at: row
                    .try_get("lease_expires_at")
                    .map_err(safe_database_error)?,
            })
        })
        .transpose()
    }

    pub async fn renew(
        &self,
        identity: &RunnerIdentity,
        fencing: JobFencing,
        lease: Duration,
    ) -> Result<bool, String> {
        let lease_ms = i64::try_from(lease.as_millis()).map_err(|error| error.to_string())?;
        sqlx::query_scalar("SELECT queue_renew_job_lease($1, $2, $3, $4, $5, $6, $7)")
            .bind(identity.runner_id)
            .bind(&identity.session_token)
            .bind(fencing.job_id)
            .bind(fencing.attempt)
            .bind(fencing.lease_epoch)
            .bind(fencing.lease_token)
            .bind(lease_ms)
            .fetch_one(&self.pool)
            .await
            .map_err(safe_database_error)
    }

    pub async fn publish_events(
        &self,
        identity: &RunnerIdentity,
        fencing: JobFencing,
        events: &Value,
    ) -> Result<u64, String> {
        let inserted: i64 =
            sqlx::query_scalar("SELECT queue_publish_events($1, $2, $3, $4, $5, $6, $7)")
                .bind(identity.runner_id)
                .bind(&identity.session_token)
                .bind(fencing.job_id)
                .bind(fencing.attempt)
                .bind(fencing.lease_epoch)
                .bind(fencing.lease_token)
                .bind(events)
                .fetch_one(&self.pool)
                .await
                .map_err(safe_database_error)?;
        u64::try_from(inserted).map_err(|error| error.to_string())
    }

    pub async fn complete(
        &self,
        identity: &RunnerIdentity,
        fencing: JobFencing,
        result: &Value,
    ) -> Result<(), String> {
        self.terminal_call("queue_complete_job", identity, fencing, result)
            .await
    }

    pub async fn acknowledge_cancellation(
        &self,
        identity: &RunnerIdentity,
        fencing: JobFencing,
        result: &Value,
    ) -> Result<(), String> {
        self.terminal_call("queue_acknowledge_cancellation", identity, fencing, result)
            .await
    }

    async fn terminal_call(
        &self,
        function: &str,
        identity: &RunnerIdentity,
        fencing: JobFencing,
        result: &Value,
    ) -> Result<(), String> {
        let sql = format!("SELECT {function}($1, $2, $3, $4, $5, $6, $7)");
        sqlx::query_scalar::<_, bool>(&sql)
            .bind(identity.runner_id)
            .bind(&identity.session_token)
            .bind(fencing.job_id)
            .bind(fencing.attempt)
            .bind(fencing.lease_epoch)
            .bind(fencing.lease_token)
            .bind(result)
            .fetch_one(&self.pool)
            .await
            .map_err(safe_database_error)?;
        Ok(())
    }

    pub async fn fail(
        &self,
        identity: &RunnerIdentity,
        fencing: JobFencing,
        error: &str,
        result: &Value,
        retryable: bool,
        backoff_base: Duration,
        backoff_max: Duration,
    ) -> Result<String, String> {
        let base_ms = i64::try_from(backoff_base.as_millis()).map_err(|error| error.to_string())?;
        let max_ms = i64::try_from(backoff_max.as_millis()).map_err(|error| error.to_string())?;
        sqlx::query_scalar("SELECT queue_fail_job($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)")
            .bind(identity.runner_id)
            .bind(&identity.session_token)
            .bind(fencing.job_id)
            .bind(fencing.attempt)
            .bind(fencing.lease_epoch)
            .bind(fencing.lease_token)
            .bind(error)
            .bind(result)
            .bind(retryable)
            .bind(base_ms)
            .bind(max_ms)
            .fetch_one(&self.pool)
            .await
            .map_err(safe_database_error)
    }

    pub async fn read_control(
        &self,
        identity: &RunnerIdentity,
        fencing: JobFencing,
    ) -> Result<String, String> {
        sqlx::query_scalar("SELECT queue_read_control($1, $2, $3, $4, $5, $6)")
            .bind(identity.runner_id)
            .bind(&identity.session_token)
            .bind(fencing.job_id)
            .bind(fencing.attempt)
            .bind(fencing.lease_epoch)
            .bind(fencing.lease_token)
            .fetch_one(&self.pool)
            .await
            .map_err(safe_database_error)
    }

    pub async fn wait_for_wakeup(&self, timeout: Duration) -> Result<(), String> {
        let mut listener = self.listener.lock().await;
        match tokio::time::timeout(timeout, listener.recv()).await {
            Ok(result) => result.map(|_| ()).map_err(safe_database_error),
            Err(_) => Ok(()),
        }
    }
}

fn safe_database_error(error: sqlx::Error) -> String {
    format!("Postgres queue operation failed: {error}")
}
