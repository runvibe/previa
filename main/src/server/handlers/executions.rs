use crate::server::db::{DatabaseKind, DbPool};
use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use sqlx::Row;
use tokio::sync::mpsc;

use crate::server::auth::Principal;
use crate::server::errors::{
    bad_request_message_response, forbidden_response, internal_error_response, not_found_response,
};
use crate::server::execution::{
    build_e2e_snapshot_payload, build_load_snapshot_payload, spawn_broadcast_bridge,
    sse_response_from_rx,
};
use crate::server::models::{
    CancelExecutionResponse, ErrorResponse, OrchestratorSseEventData, QueueDiagnosticsResponse,
    SseMessage,
};
use crate::server::queue::config::MainQueueConfig;
use crate::server::services::pipeline_access::{PipelineAccess, can_access_optional_pipeline};
use crate::server::state::{AppState, ExecutionKind};

#[derive(Debug)]
struct FinishedExecutionSnapshot {
    finished_at_ms: i64,
    init_payload: Value,
    snapshot_payload: Value,
    terminal_event: &'static str,
    terminal_payload: Value,
}

#[derive(Debug, Clone)]
struct DurableQueueSnapshot {
    execution_id: String,
    project_id: String,
    pipeline_id: Option<String>,
    status: String,
    version: i64,
    snapshot: Value,
}

async fn load_durable_queue_snapshot(
    db: &DbPool,
    execution_id: &str,
) -> Result<Option<DurableQueueSnapshot>, sqlx::Error> {
    if db.kind() != DatabaseKind::Postgres {
        return Ok(None);
    }
    let row = db
        .query(
            "SELECT CAST(execution.id AS TEXT) AS execution_id,
                    execution.project_id, execution.pipeline_id,
                    snapshot.status, snapshot.version,
                    CAST(snapshot.snapshot_json AS TEXT) AS snapshot_json
             FROM executions execution
             JOIN execution_snapshots snapshot ON snapshot.execution_id = execution.id
             WHERE CAST(execution.id AS TEXT) = ?
             LIMIT 1",
        )
        .bind(execution_id)
        .fetch_optional(db)
        .await?;
    row.map(|row| {
        let raw = row.try_get::<String, _>("snapshot_json")?;
        Ok(DurableQueueSnapshot {
            execution_id: row.try_get("execution_id")?,
            project_id: row.try_get("project_id")?,
            pipeline_id: row.try_get("pipeline_id")?,
            status: row.try_get("status")?,
            version: row.try_get("version")?,
            snapshot: serde_json::from_str(&raw).unwrap_or(Value::Null),
        })
    })
    .transpose()
}

fn stream_durable_queue_snapshot(db: DbPool, initial: DurableQueueSnapshot) -> Response {
    let (tx, rx) = mpsc::unbounded_channel::<SseMessage>();
    tokio::spawn(async move {
        let _ = tx.send(SseMessage {
            event: "execution:init".to_owned(),
            data: json!({
                "executionId": initial.execution_id,
                "status": initial.status,
                "transport": "postgres"
            }),
        });
        let _ = tx.send(SseMessage {
            event: "execution:snapshot".to_owned(),
            data: initial.snapshot.clone(),
        });
        let execution_id = initial.execution_id;
        let mut version = initial.version;
        let mut status = initial.status;
        loop {
            if matches!(status.as_str(), "completed" | "failed" | "cancelled") {
                let _ = tx.send(SseMessage {
                    event: "execution:status".to_owned(),
                    data: json!({"executionId": execution_id, "status": status}),
                });
                return;
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let Ok(Some(next)) = load_durable_queue_snapshot(&db, &execution_id).await else {
                return;
            };
            status = next.status;
            if next.version > version {
                version = next.version;
                if tx
                    .send(SseMessage {
                        event: "execution:snapshot".to_owned(),
                        data: next.snapshot,
                    })
                    .is_err()
                {
                    return;
                }
            }
        }
    });
    sse_response_from_rx(rx)
}

fn value_to_object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => {
            let mut map = Map::new();
            map.insert("payload".to_owned(), other);
            map
        }
    }
}

async fn load_finished_e2e_snapshot(
    db: &DbPool,
    project_id: &str,
    execution_id: &str,
) -> Result<Option<FinishedExecutionSnapshot>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT status, finished_at_ms, summary_json, steps_json, errors_json
        FROM integration_history
        WHERE project_id = ? AND execution_id = ?
        LIMIT 1",
    )
    .bind(project_id)
    .bind(execution_id)
    .fetch_optional(db)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    let status = row.try_get::<String, _>("status").unwrap_or_default();
    let finished_at_ms = row.try_get::<i64, _>("finished_at_ms").unwrap_or_default();
    let summary_json = row
        .try_get::<Option<String>, _>("summary_json")
        .ok()
        .flatten();
    let steps_json = row
        .try_get::<String, _>("steps_json")
        .unwrap_or_else(|_| "[]".to_owned());
    let errors_json = row
        .try_get::<String, _>("errors_json")
        .unwrap_or_else(|_| "[]".to_owned());
    let errors = serde_json::from_str::<Vec<String>>(&errors_json).unwrap_or_default();
    let summary = summary_json.and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    let steps = serde_json::from_str::<Vec<Value>>(&steps_json).unwrap_or_default();

    let mut init_payload = Map::new();
    init_payload.insert("executionId".to_owned(), json!(execution_id));
    init_payload.insert("status".to_owned(), json!(status));

    let mut terminal_payload = summary.clone().map(value_to_object).unwrap_or_default();
    terminal_payload.insert("executionId".to_owned(), json!(execution_id));
    terminal_payload.insert("status".to_owned(), json!(status));
    terminal_payload.insert("errors".to_owned(), json!(errors));

    Ok(Some(FinishedExecutionSnapshot {
        finished_at_ms,
        init_payload: Value::Object(init_payload),
        snapshot_payload: build_e2e_snapshot_payload(
            execution_id,
            &status,
            &crate::server::models::E2eHistoryAccumulator {
                summary,
                steps,
                errors: serde_json::from_str::<Vec<String>>(&errors_json).unwrap_or_default(),
            },
        ),
        terminal_event: "pipeline:complete",
        terminal_payload: Value::Object(terminal_payload),
    }))
}

async fn load_finished_load_snapshot(
    db: &DbPool,
    project_id: &str,
    execution_id: &str,
) -> Result<Option<FinishedExecutionSnapshot>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT status, finished_at_ms, context_json, final_lines_json, final_consolidated_json, errors_json
        FROM load_history
        WHERE project_id = ? AND execution_id = ?
        LIMIT 1",
    )
    .bind(project_id)
    .bind(execution_id)
    .fetch_optional(db)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    let status = row.try_get::<String, _>("status").unwrap_or_default();
    let finished_at_ms = row.try_get::<i64, _>("finished_at_ms").unwrap_or_default();
    let context_json = row
        .try_get::<String, _>("context_json")
        .unwrap_or_else(|_| "{}".to_owned());
    let final_lines_json = row
        .try_get::<String, _>("final_lines_json")
        .unwrap_or_else(|_| "[]".to_owned());
    let final_consolidated_json = row
        .try_get::<Option<String>, _>("final_consolidated_json")
        .ok()
        .flatten();
    let errors_json = row
        .try_get::<String, _>("errors_json")
        .unwrap_or_else(|_| "[]".to_owned());

    let context_value = serde_json::from_str::<Value>(&context_json).unwrap_or(Value::Null);
    let context_object = value_to_object(context_value);
    let lines = serde_json::from_str::<Vec<Value>>(&final_lines_json).unwrap_or_default();
    let consolidated = final_consolidated_json
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .unwrap_or(Value::Null);
    let errors = serde_json::from_str::<Vec<String>>(&errors_json).unwrap_or_default();

    let mut init_payload = context_object.clone();
    init_payload.insert("executionId".to_owned(), json!(execution_id));
    init_payload.insert("status".to_owned(), json!(status));

    let mut terminal_payload = context_object;
    terminal_payload.insert("executionId".to_owned(), json!(execution_id));
    terminal_payload.insert("status".to_owned(), json!(status));
    terminal_payload.insert("lines".to_owned(), Value::Array(lines));
    terminal_payload.insert("consolidated".to_owned(), consolidated);
    terminal_payload.insert("errors".to_owned(), json!(errors));

    Ok(Some(FinishedExecutionSnapshot {
        finished_at_ms,
        init_payload: Value::Object(init_payload),
        snapshot_payload: build_load_snapshot_payload(
            execution_id,
            &status,
            Value::Object(
                terminal_payload
                    .iter()
                    .filter(|(key, _)| {
                        !matches!(
                            key.as_str(),
                            "executionId" | "status" | "lines" | "consolidated" | "errors"
                        )
                    })
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            ),
            terminal_payload
                .get("lines")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            terminal_payload.get("consolidated").cloned(),
            errors.clone(),
        ),
        terminal_event: "complete",
        terminal_payload: Value::Object(terminal_payload),
    }))
}

async fn load_finished_execution_snapshot(
    db: &DbPool,
    project_id: &str,
    execution_id: &str,
) -> Result<Option<FinishedExecutionSnapshot>, sqlx::Error> {
    let e2e = load_finished_e2e_snapshot(db, project_id, execution_id).await?;
    let load = load_finished_load_snapshot(db, project_id, execution_id).await?;

    Ok(match (e2e, load) {
        (Some(e2e), Some(load)) => {
            if load.finished_at_ms > e2e.finished_at_ms {
                Some(load)
            } else {
                Some(e2e)
            }
        }
        (Some(e2e), None) => Some(e2e),
        (None, Some(load)) => Some(load),
        (None, None) => None,
    })
}

async fn load_finished_execution_snapshot_by_id(
    db: &DbPool,
    execution_id: &str,
) -> Result<Option<FinishedExecutionSnapshot>, sqlx::Error> {
    let project_rows = sqlx::query(
        "SELECT project_id, finished_at_ms FROM integration_history WHERE execution_id = ?
        UNION ALL
        SELECT project_id, finished_at_ms FROM load_history WHERE execution_id = ?
        ORDER BY finished_at_ms DESC
        LIMIT 1",
    )
    .bind(execution_id)
    .bind(execution_id)
    .fetch_optional(db)
    .await?;

    let Some(row) = project_rows else {
        return Ok(None);
    };
    let project_id = row.try_get::<String, _>("project_id").unwrap_or_default();
    load_finished_execution_snapshot(db, &project_id, execution_id).await
}

async fn load_finished_execution_pipeline_ref(
    db: &DbPool,
    project_id: &str,
    execution_id: &str,
) -> Result<Option<Option<String>>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT pipeline_id, finished_at_ms FROM integration_history WHERE project_id = ? AND execution_id = ?
        UNION ALL
        SELECT pipeline_id, finished_at_ms FROM load_history WHERE project_id = ? AND execution_id = ?
        ORDER BY finished_at_ms DESC
        LIMIT 1",
    )
    .bind(project_id)
    .bind(execution_id)
    .bind(project_id)
    .bind(execution_id)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|row| {
        row.try_get::<Option<String>, _>("pipeline_id")
            .ok()
            .flatten()
    }))
}

async fn load_finished_execution_project_pipeline_ref(
    db: &DbPool,
    execution_id: &str,
) -> Result<Option<(String, Option<String>)>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT project_id, pipeline_id, finished_at_ms FROM integration_history WHERE execution_id = ?
        UNION ALL
        SELECT project_id, pipeline_id, finished_at_ms FROM load_history WHERE execution_id = ?
        ORDER BY finished_at_ms DESC
        LIMIT 1",
    )
    .bind(execution_id)
    .bind(execution_id)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|row| {
        (
            row.try_get::<String, _>("project_id").unwrap_or_default(),
            row.try_get::<Option<String>, _>("pipeline_id")
                .ok()
                .flatten(),
        )
    }))
}

async fn stream_active_execution(
    execution: std::sync::Arc<crate::server::state::ExecutionCtx>,
) -> Response {
    let skip_execution_init = match execution.kind {
        ExecutionKind::E2e | ExecutionKind::Load => true,
    };
    let (tx, rx) = mpsc::unbounded_channel::<SseMessage>();
    let init_payload = execution.init_payload.get().await;
    let _ = tx.send(SseMessage {
        event: "execution:init".to_owned(),
        data: init_payload,
    });
    let _ = tx.send(SseMessage {
        event: "execution:snapshot".to_owned(),
        data: execution.snapshot_payload.get().await,
    });
    spawn_broadcast_bridge(execution.sse_tx.subscribe(), tx, skip_execution_init);
    sse_response_from_rx(rx)
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{projectId}/executions/{executionId}",
    params(
        ("projectId" = String, Path, description = "ID do projeto"),
        ("executionId" = String, Path, description = "ID da execução retornado no evento SSE execution:init")
    ),
    responses(
        (
            status = 200,
            description = "Stream SSE da execução com replay inicial: execution:init, execution:snapshot e depois eventos ao vivo ou evento terminal.",
            content_type = "text/event-stream",
            body = OrchestratorSseEventData
        ),
        (
            status = 400,
            description = "Parâmetro inválido",
            body = ErrorResponse
        ),
        (
            status = 404,
            description = "Execução não encontrada para o projeto",
            body = ErrorResponse
        ),
        (
            status = 500,
            description = "Erro interno ao recuperar execução",
            body = ErrorResponse
        )
    )
)]
pub async fn stream_execution(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((project_id, execution_id)): Path<(String, String)>,
) -> Response {
    let project_id = project_id.trim().to_owned();
    let execution_id = execution_id.trim().to_owned();
    if project_id.is_empty() {
        return bad_request_message_response("projectId cannot be empty");
    }
    if execution_id.is_empty() {
        return bad_request_message_response("executionId cannot be empty");
    }

    let execution = {
        let executions = state.executions.read().await;
        executions.get(&execution_id).cloned()
    };

    if let Some(execution) = execution {
        if execution.project_id != project_id {
            return not_found_response("execution not found for project");
        }
        match can_access_optional_pipeline(
            &state.db,
            &project_id,
            execution.pipeline_id.as_deref(),
            &principal,
            PipelineAccess::Read,
        )
        .await
        {
            Ok(true) => {}
            Ok(false) => return not_found_response("execution not found for project"),
            Err(err) => {
                return internal_error_response(format!("failed to authorize execution: {err}"));
            }
        }

        return stream_active_execution(execution).await;
    }

    match load_durable_queue_snapshot(&state.db, &execution_id).await {
        Ok(Some(snapshot)) if snapshot.project_id == project_id => {
            match can_access_optional_pipeline(
                &state.db,
                &project_id,
                snapshot.pipeline_id.as_deref(),
                &principal,
                PipelineAccess::Read,
            )
            .await
            {
                Ok(true) => return stream_durable_queue_snapshot(state.db.clone(), snapshot),
                Ok(false) => return not_found_response("execution not found for project"),
                Err(error) => {
                    return internal_error_response(format!(
                        "failed to authorize execution: {error}"
                    ));
                }
            }
        }
        Ok(Some(_)) | Ok(None) => {}
        Err(error) => {
            return internal_error_response(format!(
                "failed to load durable execution snapshot: {error}"
            ));
        }
    }

    let pipeline_id =
        match load_finished_execution_pipeline_ref(&state.db, &project_id, &execution_id).await {
            Ok(pipeline_id) => pipeline_id,
            Err(err) => {
                return internal_error_response(format!("failed to load execution history: {err}"));
            }
        };
    match can_access_optional_pipeline(
        &state.db,
        &project_id,
        pipeline_id.as_ref().and_then(|value| value.as_deref()),
        &principal,
        PipelineAccess::Read,
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => return not_found_response("execution not found for project"),
        Err(err) => {
            return internal_error_response(format!("failed to authorize execution: {err}"));
        }
    }

    let snapshot =
        match load_finished_execution_snapshot(&state.db, &project_id, &execution_id).await {
            Ok(snapshot) => snapshot,
            Err(err) => {
                return internal_error_response(format!("failed to load execution history: {err}"));
            }
        };

    let Some(snapshot) = snapshot else {
        return not_found_response("execution not found for project");
    };

    let (tx, rx) = mpsc::unbounded_channel::<SseMessage>();
    let _ = tx.send(SseMessage {
        event: "execution:init".to_owned(),
        data: snapshot.init_payload,
    });
    let _ = tx.send(SseMessage {
        event: "execution:snapshot".to_owned(),
        data: snapshot.snapshot_payload,
    });
    let _ = tx.send(SseMessage {
        event: snapshot.terminal_event.to_owned(),
        data: snapshot.terminal_payload,
    });
    drop(tx);

    sse_response_from_rx(rx)
}

#[utoipa::path(
    get,
    path = "/api/v1/executions/{executionId}/events",
    params(
        ("executionId" = String, Path, description = "ID da execução")
    ),
    responses(
        (
            status = 200,
            description = "Stream SSE da execução com replay inicial e eventos ao vivo ou terminal.",
            content_type = "text/event-stream",
            body = OrchestratorSseEventData
        ),
        (
            status = 400,
            description = "Parâmetro inválido",
            body = ErrorResponse
        ),
        (
            status = 404,
            description = "Execução não encontrada",
            body = ErrorResponse
        )
    )
)]
pub async fn stream_execution_events(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(execution_id): Path<String>,
) -> Response {
    let execution_id = execution_id.trim().to_owned();
    if execution_id.is_empty() {
        return bad_request_message_response("executionId cannot be empty");
    }

    let execution = {
        let executions = state.executions.read().await;
        executions.get(&execution_id).cloned()
    };
    if let Some(execution) = execution {
        match can_access_optional_pipeline(
            &state.db,
            &execution.project_id,
            execution.pipeline_id.as_deref(),
            &principal,
            PipelineAccess::Read,
        )
        .await
        {
            Ok(true) => {}
            Ok(false) => return not_found_response("execution not found"),
            Err(err) => {
                return internal_error_response(format!("failed to authorize execution: {err}"));
            }
        }
        return stream_active_execution(execution).await;
    }

    match load_durable_queue_snapshot(&state.db, &execution_id).await {
        Ok(Some(snapshot)) => {
            match can_access_optional_pipeline(
                &state.db,
                &snapshot.project_id,
                snapshot.pipeline_id.as_deref(),
                &principal,
                PipelineAccess::Read,
            )
            .await
            {
                Ok(true) => return stream_durable_queue_snapshot(state.db.clone(), snapshot),
                Ok(false) => return not_found_response("execution not found"),
                Err(error) => {
                    return internal_error_response(format!(
                        "failed to authorize execution: {error}"
                    ));
                }
            }
        }
        Ok(None) => {}
        Err(error) => {
            return internal_error_response(format!(
                "failed to load durable execution snapshot: {error}"
            ));
        }
    }

    let execution_ref =
        match load_finished_execution_project_pipeline_ref(&state.db, &execution_id).await {
            Ok(value) => value,
            Err(err) => {
                return internal_error_response(format!("failed to load execution history: {err}"));
            }
        };
    let Some((project_id, pipeline_id)) = execution_ref else {
        return not_found_response("execution not found");
    };
    match can_access_optional_pipeline(
        &state.db,
        &project_id,
        pipeline_id.as_deref(),
        &principal,
        PipelineAccess::Read,
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => return not_found_response("execution not found"),
        Err(err) => {
            return internal_error_response(format!("failed to authorize execution: {err}"));
        }
    }

    let snapshot = match load_finished_execution_snapshot_by_id(&state.db, &execution_id).await {
        Ok(snapshot) => snapshot,
        Err(err) => {
            return internal_error_response(format!("failed to load execution history: {err}"));
        }
    };
    let Some(snapshot) = snapshot else {
        return not_found_response("execution not found");
    };

    let (tx, rx) = mpsc::unbounded_channel::<SseMessage>();
    let _ = tx.send(SseMessage {
        event: "execution:init".to_owned(),
        data: snapshot.init_payload,
    });
    let _ = tx.send(SseMessage {
        event: "execution:snapshot".to_owned(),
        data: snapshot.snapshot_payload,
    });
    let _ = tx.send(SseMessage {
        event: snapshot.terminal_event.to_owned(),
        data: snapshot.terminal_payload,
    });
    drop(tx);

    sse_response_from_rx(rx)
}

#[utoipa::path(
    post,
    path = "/api/v1/executions/{executionId}/cancel",
    params(
        ("executionId" = String, Path, description = "ID da execução retornado no evento SSE execution:init")
    ),
    responses(
        (
            status = 202,
            description = "Cancelamento solicitado",
            body = CancelExecutionResponse
        ),
        (
            status = 400,
            description = "Parâmetro inválido",
            body = ErrorResponse
        ),
        (
            status = 404,
            description = "Execução não encontrada ou já finalizada",
            body = ErrorResponse
        )
    )
)]
pub async fn cancel_execution(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(execution_id): Path<String>,
) -> Response {
    let execution_id = execution_id.trim().to_owned();
    if execution_id.is_empty() {
        return bad_request_message_response("executionId cannot be empty");
    }

    let execution = {
        let executions = state.executions.read().await;
        executions.get(&execution_id).cloned()
    };

    let Some(execution) = execution else {
        let snapshot = match load_durable_queue_snapshot(&state.db, &execution_id).await {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => return not_found_response("execution not found or already finished"),
            Err(error) => {
                return internal_error_response(format!(
                    "failed to load durable execution: {error}"
                ));
            }
        };
        match can_access_optional_pipeline(
            &state.db,
            &snapshot.project_id,
            snapshot.pipeline_id.as_deref(),
            &principal,
            PipelineAccess::Run,
        )
        .await
        {
            Ok(true) => {}
            Ok(false) => return forbidden_response("execution access denied"),
            Err(error) => {
                return internal_error_response(format!("failed to authorize execution: {error}"));
            }
        }
        if matches!(
            snapshot.status.as_str(),
            "completed" | "failed" | "cancelled"
        ) {
            return not_found_response("execution not found or already finished");
        }
        let updated = match state
            .db
            .query(
                "UPDATE executions
                 SET desired_status = 'cancelled',
                     status = 'cancel_requested',
                     updated_at = CURRENT_TIMESTAMP
                 WHERE CAST(id AS TEXT) = ?
                   AND status NOT IN ('completed', 'failed', 'cancelled')",
            )
            .bind(&execution_id)
            .execute(&state.db)
            .await
        {
            Ok(result) => result.rows_affected() > 0,
            Err(error) => {
                return internal_error_response(format!(
                    "failed to cancel durable execution: {error}"
                ));
            }
        };
        let _ = state
            .db
            .query(
                "UPDATE execution_jobs
                 SET status = 'cancelled',
                     finished_at = CURRENT_TIMESTAMP,
                     updated_at = CURRENT_TIMESTAMP
                 WHERE CAST(execution_id AS TEXT) = ?
                   AND status IN ('queued', 'retry_wait')",
            )
            .bind(&execution_id)
            .execute(&state.db)
            .await;
        let _ = state
            .db
            .query("SELECT pg_notify('previa_control', ?)")
            .bind(&execution_id)
            .execute(&state.db)
            .await;
        return (
            StatusCode::ACCEPTED,
            Json(CancelExecutionResponse {
                execution_id,
                cancelled: updated,
                already_cancelled: snapshot.status == "cancel_requested",
                message: "cancellation requested".to_owned(),
            }),
        )
            .into_response();
    };
    match can_access_optional_pipeline(
        &state.db,
        &execution.project_id,
        execution.pipeline_id.as_deref(),
        &principal,
        PipelineAccess::Run,
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => return forbidden_response("execution access denied"),
        Err(err) => {
            return internal_error_response(format!("failed to authorize execution: {err}"));
        }
    }

    let already_cancelled = execution.cancel.is_cancelled();
    execution.cancel.cancel();

    (
        StatusCode::ACCEPTED,
        Json(CancelExecutionResponse {
            execution_id,
            cancelled: true,
            already_cancelled,
            message: if already_cancelled {
                "cancellation already requested".to_owned()
            } else {
                "cancellation requested".to_owned()
            },
        }),
    )
        .into_response()
}

#[utoipa::path(
    get,
    path = "/api/v1/queue/diagnostics",
    responses(
        (
            status = 200,
            description = "Diagnóstico seguro da fila Postgres",
            body = QueueDiagnosticsResponse
        ),
        (
            status = 500,
            description = "Falha ao consultar a fila",
            body = ErrorResponse
        )
    )
)]
pub async fn queue_diagnostics(State(state): State<AppState>) -> Response {
    let config = match MainQueueConfig::from_env() {
        Ok(config) => config,
        Err(error) => return internal_error_response(error),
    };
    let row = match state
        .db
        .query(
            "SELECT
                (SELECT protocol_version FROM queue_protocol WHERE id = 1) AS protocol_version,
                (SELECT count(*) FROM execution_jobs WHERE status = 'queued') AS queued_jobs,
                (SELECT count(*) FROM execution_jobs WHERE status IN ('leased', 'running')) AS active_jobs,
                (SELECT count(*) FROM execution_jobs WHERE status = 'retry_wait') AS retry_wait_jobs,
                (SELECT count(*) FROM execution_jobs WHERE status = 'dead_letter') AS dead_letter_jobs,
                COALESCE((
                    SELECT (EXTRACT(EPOCH FROM (CURRENT_TIMESTAMP - min(available_at))) * 1000)::DOUBLE PRECISION
                    FROM execution_jobs
                    WHERE status = 'queued' AND available_at <= CURRENT_TIMESTAMP
                ), 0) AS oldest_eligible_age_ms,
                (
                    SELECT count(*)
                    FROM execution_events event
                    LEFT JOIN execution_snapshots snapshot
                      ON snapshot.execution_id = event.execution_id
                    WHERE event.id > COALESCE(snapshot.last_event_id, 0)
                ) AS event_backlog,
                (SELECT count(*) FROM runner_instances WHERE status IN ('ready', 'busy')) AS ready_runners,
                (SELECT count(*) FROM runner_instances WHERE status = 'stale') AS stale_runners",
        )
        .fetch_one(&state.db)
        .await
    {
        Ok(row) => row,
        Err(error) => {
            return internal_error_response(format!(
                "failed to query Postgres queue diagnostics: {error}"
            ));
        }
    };
    Json(QueueDiagnosticsResponse {
        protocol_version: row.get("protocol_version"),
        queued_jobs: row.get("queued_jobs"),
        active_jobs: row.get("active_jobs"),
        retry_wait_jobs: row.get("retry_wait_jobs"),
        dead_letter_jobs: row.get("dead_letter_jobs"),
        oldest_eligible_age_ms: row
            .try_get::<f64, _>("oldest_eligible_age_ms")
            .unwrap_or_default()
            .max(0.0) as i64,
        event_backlog: row.get("event_backlog"),
        ready_runners: row.get("ready_runners"),
        stale_runners: row.get("stale_runners"),
        runner_stale_after_ms: config.runner_stale_after.as_millis() as u64,
        job_lease_ms: config.job_lease.as_millis() as u64,
        projection_poll_interval_ms: config.projection_poll_interval.as_millis() as u64,
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use axum::response::Response;
    use serde_json::json;
    use tokio::sync::{RwLock, broadcast};
    use tokio_stream::StreamExt;
    use tokio_util::sync::CancellationToken;
    use tower::ServiceExt;

    use crate::server::build_app;
    use crate::server::db::{save_e2e_history, save_load_history};
    use crate::server::execution::ExecutionScheduler;
    use crate::server::execution::scheduler::SharedValue;
    use crate::server::mcp::models::McpConfig;
    use crate::server::models::{E2eHistoryWrite, HistoryMetadata, LoadHistoryWrite, SseMessage};
    use crate::server::state::{AppState, ExecutionCtx, ExecutionKind};

    #[tokio::test]
    async fn active_e2e_execution_stream_replays_snapshot_before_live_events() {
        let state = test_state().await;
        let (sse_tx, _) = broadcast::channel(16);
        {
            let mut executions = state.executions.write().await;
            executions.insert(
                "exec-1".to_owned(),
                Arc::new(ExecutionCtx {
                    cancel: CancellationToken::new(),
                    project_id: "project-1".to_owned(),
                    pipeline_id: Some("pipe-1".to_owned()),
                    kind: ExecutionKind::E2e,
                    sse_tx: sse_tx.clone(),
                    init_payload: SharedValue::new(json!({
                        "executionId": "exec-1",
                        "status": "running"
                    })),
                    snapshot_payload: SharedValue::new(json!({
                        "executionId": "exec-1",
                        "status": "running",
                        "kind": "e2e",
                        "steps": [
                            { "stepId": "step-1", "status": "success" }
                        ],
                        "summary": null,
                        "errors": []
                    })),
                }),
            );
        }
        let app = test_app(state.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/projects/project-1/executions/exec-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let mut body = response.into_body().into_data_stream();
        let init_chunk = next_chunk_text(&mut body).await;
        let snapshot_chunk = next_chunk_text(&mut body).await;

        let _ = sse_tx.send(SseMessage {
            event: "step:result".to_owned(),
            data: json!({ "stepId": "step-2", "status": "running" }),
        });
        let live_chunk = next_chunk_text(&mut body).await;

        assert!(init_chunk.contains("event: execution:init"));
        assert!(snapshot_chunk.contains("event: execution:snapshot"));
        assert!(snapshot_chunk.contains("\"kind\":\"e2e\""));
        assert!(snapshot_chunk.contains("\"stepId\":\"step-1\""));
        assert!(live_chunk.contains("event: step:result"));
        assert!(live_chunk.contains("\"stepId\":\"step-2\""));
    }

    #[tokio::test]
    async fn active_load_execution_stream_replays_snapshot_before_live_events() {
        let state = test_state().await;
        let (sse_tx, _) = broadcast::channel(16);
        {
            let mut executions = state.executions.write().await;
            executions.insert(
                "exec-2".to_owned(),
                Arc::new(ExecutionCtx {
                    cancel: CancellationToken::new(),
                    project_id: "project-1".to_owned(),
                    pipeline_id: Some("pipe-1".to_owned()),
                    kind: ExecutionKind::Load,
                    sse_tx: sse_tx.clone(),
                    init_payload: SharedValue::new(json!({
                        "executionId": "exec-2",
                        "status": "running"
                    })),
                    snapshot_payload: SharedValue::new(json!({
                        "executionId": "exec-2",
                        "status": "running",
                        "kind": "load",
                        "context": { "nodesFound": 1 },
                        "lines": [
                            { "node": "runner-1", "runnerEvent": "metrics", "receivedAt": 1, "payload": { "ok": true } }
                        ],
                        "consolidated": { "totalOk": 10, "totalError": 0 },
                        "errors": []
                    })),
                }),
            );
        }
        let app = test_app(state.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/projects/project-1/executions/exec-2")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let mut body = response.into_body().into_data_stream();
        let init_chunk = next_chunk_text(&mut body).await;
        let snapshot_chunk = next_chunk_text(&mut body).await;

        let _ = sse_tx.send(SseMessage {
            event: "metrics".to_owned(),
            data: json!({ "lines": [], "consolidated": { "totalOk": 20, "totalError": 0 } }),
        });
        let live_chunk = next_chunk_text(&mut body).await;

        assert!(init_chunk.contains("event: execution:init"));
        assert!(snapshot_chunk.contains("event: execution:snapshot"));
        assert!(snapshot_chunk.contains("\"kind\":\"load\""));
        assert!(snapshot_chunk.contains("\"totalOk\":10"));
        assert!(live_chunk.contains("event: metrics"));
        assert!(live_chunk.contains("\"totalOk\":20"));
    }

    #[tokio::test]
    async fn finished_e2e_execution_stream_includes_snapshot_before_terminal_event() {
        let state = test_state().await;
        save_e2e_history(
            &state.db,
            E2eHistoryWrite {
                id: "hist-1".to_owned(),
                execution_id: "exec-finished-e2e".to_owned(),
                transaction_id: None,
                metadata: HistoryMetadata {
                    project_id: Some("project-1".to_owned()),
                    pipeline_index: Some(0),
                },
                pipeline_id: Some("pipe-1".to_owned()),
                pipeline_name: "Pipeline".to_owned(),
                selected_base_url_key: None,
                status: "success".to_owned(),
                started_at_ms: 1,
                finished_at_ms: 2,
                duration_ms: 1,
                summary: Some(json!({ "passed": 1, "failed": 0 })),
                steps: vec![json!({ "stepId": "step-1", "status": "success" })],
                errors: Vec::new(),
                request: json!({}),
            },
        )
        .await
        .expect("save e2e history");
        let app = test_app(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/projects/project-1/executions/exec-finished-e2e")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = collect_stream_body(response).await;
        assert_in_order(
            &body,
            &[
                "event: execution:init",
                "event: execution:snapshot",
                "event: pipeline:complete",
            ],
        );
        assert!(body.contains("\"kind\":\"e2e\""));
        assert!(body.contains("\"stepId\":\"step-1\""));
    }

    #[tokio::test]
    async fn finished_load_execution_stream_includes_snapshot_before_terminal_event() {
        let state = test_state().await;
        save_load_history(
            &state.db,
            LoadHistoryWrite {
                id: "hist-2".to_owned(),
                execution_id: "exec-finished-load".to_owned(),
                transaction_id: None,
                metadata: HistoryMetadata {
                    project_id: Some("project-1".to_owned()),
                    pipeline_index: Some(0),
                },
                pipeline_id: Some("pipe-1".to_owned()),
                pipeline_name: "Pipeline".to_owned(),
                selected_base_url_key: None,
                status: "success".to_owned(),
                started_at_ms: 1,
                finished_at_ms: 2,
                duration_ms: 1,
                requested_config: json!({ "totalRequests": 10, "concurrency": 1, "rampUpSeconds": 0.0 }),
                final_consolidated: Some(json!({ "totalOk": 10, "totalError": 0 })),
                final_lines: vec![json!({
                    "node": "runner-1",
                    "runnerEvent": "metrics",
                    "receivedAt": 1,
                    "payload": { "ok": true }
                })],
                errors: Vec::new(),
                request: json!({}),
                context: json!({ "nodesFound": 1, "nodesUsed": 1 }),
            },
        )
        .await
        .expect("save load history");
        let app = test_app(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/projects/project-1/executions/exec-finished-load")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = collect_stream_body(response).await;
        assert_in_order(
            &body,
            &[
                "event: execution:init",
                "event: execution:snapshot",
                "event: complete",
            ],
        );
        assert!(body.contains("\"kind\":\"load\""));
        assert!(body.contains("\"nodesFound\":1"));
        assert!(body.contains("\"totalOk\":10"));
    }

    async fn test_state() -> AppState {
        let db = crate::server::db::DbPool::connect_test_sqlite("sqlite::memory:", 1)
            .await
            .expect("sqlite memory db");
        sqlx::migrate!("./migrations/sqlite")
            .run(db.pool())
            .await
            .expect("migrations");

        AppState {
            client: reqwest::Client::new(),
            db,
            context_name: "default".to_owned(),
            runner_auth_key: None,
            auth: crate::server::auth::AuthRuntime::anonymous(),
            rps_per_node: 1000,
            scheduler: ExecutionScheduler::new(Default::default()),
            executions: Arc::new(RwLock::new(HashMap::new())),
            e2e_queues: Arc::new(RwLock::new(HashMap::new())),
            mcp_sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn test_app(state: AppState) -> Router {
        build_app(
            state,
            &McpConfig {
                enabled: false,
                path: "/mcp".to_owned(),
            },
        )
    }

    async fn next_chunk_text(
        body: &mut (impl tokio_stream::Stream<Item = Result<axum::body::Bytes, axum::Error>> + Unpin),
    ) -> String {
        let chunk = tokio::time::timeout(Duration::from_secs(2), body.next())
            .await
            .expect("chunk timeout")
            .expect("chunk exists")
            .expect("body chunk");
        String::from_utf8(chunk.to_vec()).expect("utf8 chunk")
    }

    async fn collect_stream_body(response: Response) -> String {
        let mut stream = response.into_body().into_data_stream();
        let mut chunks = Vec::new();
        while let Some(item) = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .expect("stream timeout")
        {
            chunks.push(item.expect("body chunk"));
        }
        String::from_utf8(
            chunks
                .into_iter()
                .flat_map(|chunk| chunk.to_vec())
                .collect(),
        )
        .expect("utf8 body")
    }

    fn assert_in_order(body: &str, snippets: &[&str]) {
        let mut last_index = 0;
        for snippet in snippets {
            let found = body[last_index..]
                .find(snippet)
                .map(|index| index + last_index)
                .unwrap_or_else(|| panic!("missing snippet: {snippet}\nbody:\n{body}"));
            last_index = found + snippet.len();
        }
    }
}
