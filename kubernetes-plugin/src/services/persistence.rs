use std::borrow::Cow;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sqlx::any::{AnyConnectOptions, AnyPoolOptions};
use sqlx::{Any, Executor, Pool, Row};
use thiserror::Error;

use crate::models::{ReservationCreateRequest, ReservationStatus, RunnerLifecycleState};

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("unsupported database URL scheme '{0}'")]
    UnsupportedDatabase(String),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct PersistedReservation {
    pub request: ReservationCreateRequest,
    pub status: ReservationStatus,
    pub created_at: String,
    pub token: String,
    pub physical_runner_count: usize,
}

#[derive(Debug, Clone)]
pub struct PersistedPhysicalRunner {
    pub id: String,
    pub endpoint: String,
    pub physical_reservation_id: String,
    pub logical_reservation_id: Option<String>,
    pub state: RunnerLifecycleState,
    pub idle_since: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct PersistedReservationState {
    pub reservations: Vec<PersistedReservation>,
    pub runners: Vec<PersistedPhysicalRunner>,
}

#[async_trait]
pub trait ReservationPersistence: Send + Sync {
    async fn load_state(&self) -> Result<PersistedReservationState, PersistenceError>;

    async fn save_state(&self, state: PersistedReservationState) -> Result<(), PersistenceError>;
}

#[derive(Clone)]
pub struct SqlReservationPersistence {
    pool: Pool<Any>,
    kind: DatabaseKind,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PersistedReservationRequestPayload {
    request: ReservationCreateRequest,
    physical_runner_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DatabaseKind {
    Sqlite,
    Postgres,
}

impl DatabaseKind {
    fn from_url(database_url: &str) -> Result<Self, PersistenceError> {
        let scheme = database_url
            .split_once(':')
            .map(|(scheme, _)| scheme)
            .unwrap_or_default();
        match scheme {
            "sqlite" => Ok(Self::Sqlite),
            "postgres" | "postgresql" => Ok(Self::Postgres),
            _ => Err(PersistenceError::UnsupportedDatabase(scheme.to_owned())),
        }
    }
}

impl SqlReservationPersistence {
    pub async fn connect(database_url: &str) -> Result<Self, PersistenceError> {
        sqlx::any::install_default_drivers();
        let kind = DatabaseKind::from_url(database_url)?;
        if kind == DatabaseKind::Sqlite {
            create_sqlite_file_if_missing(database_url)?;
        }
        let options = database_url.parse::<AnyConnectOptions>()?;
        let mut pool_options = AnyPoolOptions::new().max_connections(5);
        if kind == DatabaseKind::Sqlite {
            pool_options = pool_options.after_connect(|conn, _meta| {
                Box::pin(async move {
                    conn.execute("PRAGMA foreign_keys = ON").await?;
                    Ok(())
                })
            });
        }
        let pool = pool_options.connect_with(options).await?;
        let persistence = Self { pool, kind };
        persistence.migrate().await?;
        Ok(persistence)
    }

    async fn migrate(&self) -> Result<(), PersistenceError> {
        self.execute(
            "CREATE TABLE IF NOT EXISTS plugin_runner_reservations (
                reservation_id TEXT PRIMARY KEY NOT NULL,
                execution_id TEXT NOT NULL,
                pipeline_id TEXT NOT NULL,
                reservation_status TEXT NOT NULL,
                request_json TEXT NOT NULL,
                status_json TEXT NOT NULL,
                token TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
        )
        .await?;
        self.execute(
            "CREATE INDEX IF NOT EXISTS idx_plugin_runner_reservations_status
                ON plugin_runner_reservations(reservation_status)",
        )
        .await?;
        self.execute(
            "CREATE TABLE IF NOT EXISTS plugin_physical_runners (
                endpoint TEXT PRIMARY KEY NOT NULL,
                runner_id TEXT NOT NULL,
                physical_reservation_id TEXT NOT NULL,
                logical_reservation_id TEXT,
                state TEXT NOT NULL,
                idle_since TEXT,
                updated_at TEXT NOT NULL
            )",
        )
        .await?;
        self.execute(
            "CREATE INDEX IF NOT EXISTS idx_plugin_physical_runners_physical_reservation
                ON plugin_physical_runners(physical_reservation_id)",
        )
        .await?;
        Ok(())
    }

    async fn execute(&self, sql: &str) -> Result<(), PersistenceError> {
        sqlx::query::<Any>(self.sql(sql).as_ref())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    fn sql<'q>(&self, sql: &'q str) -> Cow<'q, str> {
        match self.kind {
            DatabaseKind::Sqlite => Cow::Borrowed(sql),
            DatabaseKind::Postgres => Cow::Owned(postgres_sql(sql)),
        }
    }
}

#[async_trait]
impl ReservationPersistence for SqlReservationPersistence {
    async fn load_state(&self) -> Result<PersistedReservationState, PersistenceError> {
        let reservation_rows = sqlx::query::<Any>(
            self.sql(
                "SELECT request_json, status_json, token, created_at
            FROM plugin_runner_reservations
            WHERE reservation_status NOT IN ('failed', 'cancelled', 'expired', 'terminating')",
            )
            .as_ref(),
        )
        .fetch_all(&self.pool)
        .await?;
        let mut reservations = Vec::new();
        for row in reservation_rows {
            let request_json = row.try_get::<String, _>("request_json")?;
            let (request, physical_runner_count) =
                match serde_json::from_str::<PersistedReservationRequestPayload>(&request_json) {
                    Ok(payload) => (payload.request, payload.physical_runner_count),
                    Err(_) => (
                        serde_json::from_str::<ReservationCreateRequest>(&request_json)?,
                        0,
                    ),
                };
            reservations.push(PersistedReservation {
                request,
                status: serde_json::from_str(row.try_get::<String, _>("status_json")?.as_str())?,
                created_at: row.try_get("created_at")?,
                token: row.try_get("token")?,
                physical_runner_count,
            });
        }

        let runner_rows = sqlx::query::<Any>(
            self.sql(
                "SELECT runner_id, endpoint, physical_reservation_id, logical_reservation_id,
                state, idle_since
            FROM plugin_physical_runners",
            )
            .as_ref(),
        )
        .fetch_all(&self.pool)
        .await?;
        let mut runners = Vec::new();
        for row in runner_rows {
            runners.push(PersistedPhysicalRunner {
                id: row.try_get("runner_id")?,
                endpoint: row.try_get("endpoint")?,
                physical_reservation_id: row.try_get("physical_reservation_id")?,
                logical_reservation_id: row.try_get("logical_reservation_id").ok().flatten(),
                state: serde_json::from_str::<RunnerLifecycleState>(
                    format!("\"{}\"", row.try_get::<String, _>("state")?).as_str(),
                )?,
                idle_since: row.try_get("idle_since").ok().flatten(),
            });
        }

        Ok(PersistedReservationState {
            reservations,
            runners,
        })
    }

    async fn save_state(&self, state: PersistedReservationState) -> Result<(), PersistenceError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query::<Any>(self.sql("DELETE FROM plugin_runner_reservations").as_ref())
            .execute(&mut *tx)
            .await?;
        sqlx::query::<Any>(self.sql("DELETE FROM plugin_physical_runners").as_ref())
            .execute(&mut *tx)
            .await?;

        for reservation in state.reservations {
            let request_json = serde_json::to_string(&PersistedReservationRequestPayload {
                request: reservation.request.clone(),
                physical_runner_count: reservation.physical_runner_count,
            })?;
            let status_json = serde_json::to_string(&reservation.status)?;
            let reservation_status = serde_json::to_string(&reservation.status.status)?
                .trim_matches('"')
                .to_owned();
            sqlx::query::<Any>(
                self.sql(
                    "INSERT INTO plugin_runner_reservations (
                        reservation_id, execution_id, pipeline_id, reservation_status,
                        request_json, status_json, token, created_at, updated_at
                    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .as_ref(),
            )
            .bind(&reservation.status.reservation_id)
            .bind(&reservation.request.execution_id)
            .bind(&reservation.request.pipeline_id)
            .bind(&reservation_status)
            .bind(&request_json)
            .bind(&status_json)
            .bind(&reservation.token)
            .bind(&reservation.created_at)
            .bind(&reservation.status.updated_at)
            .execute(&mut *tx)
            .await?;
        }

        for runner in state.runners {
            let state = serde_json::to_string(&runner.state)?
                .trim_matches('"')
                .to_owned();
            sqlx::query::<Any>(
                self.sql(
                    "INSERT INTO plugin_physical_runners (
                        endpoint, runner_id, physical_reservation_id,
                        logical_reservation_id, state, idle_since, updated_at
                    ) VALUES (?, ?, ?, ?, ?, ?, ?)",
                )
                .as_ref(),
            )
            .bind(&runner.endpoint)
            .bind(&runner.id)
            .bind(&runner.physical_reservation_id)
            .bind(&runner.logical_reservation_id)
            .bind(&state)
            .bind(&runner.idle_since)
            .bind(chrono::Utc::now().to_rfc3339())
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }
}

fn create_sqlite_file_if_missing(database_url: &str) -> Result<(), sqlx::Error> {
    let Some(path) = database_url.strip_prefix("sqlite://") else {
        return Ok(());
    };
    let path = path.split_once('?').map(|(path, _)| path).unwrap_or(path);
    if path.is_empty() || path == ":memory:" {
        return Ok(());
    }

    let path = Path::new(path);
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|err| {
            sqlx::Error::Configuration(
                format!("failed to create sqlite database directory: {err}").into(),
            )
        })?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map(|_| ())
        .map_err(|err| {
            sqlx::Error::Configuration(
                format!("failed to create sqlite database file: {err}").into(),
            )
        })
}

fn postgres_sql(sql: &str) -> String {
    static CACHE: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(rewritten) = cache.lock().expect("sql cache lock").get(sql).copied() {
        return rewritten.to_owned();
    }

    let rewritten = rewrite_question_placeholders(sql);
    let leaked = Box::leak(rewritten.into_boxed_str());
    cache
        .lock()
        .expect("sql cache lock")
        .insert(sql.to_owned(), leaked);
    leaked.to_owned()
}

fn rewrite_question_placeholders(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut placeholder = 1usize;
    let mut chars = sql.chars().peekable();
    let mut in_single_quote = false;

    while let Some(ch) = chars.next() {
        if ch == '\'' {
            out.push(ch);
            if in_single_quote && matches!(chars.peek(), Some('\'')) {
                out.push(chars.next().expect("peeked quote"));
                continue;
            }
            in_single_quote = !in_single_quote;
            continue;
        }

        if ch == '?' && !in_single_quote {
            out.push('$');
            out.push_str(&placeholder.to_string());
            placeholder += 1;
        } else {
            out.push(ch);
        }
    }

    out
}
