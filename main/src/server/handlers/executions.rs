use crate::server::db::DbPool;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use sqlx::Row;
use tokio::sync::mpsc;

use crate::server::errors::{
    bad_request_message_response, internal_error_response, not_found_response,
};
use crate::server::execution::{
    build_e2e_snapshot_payload, build_load_snapshot_payload, spawn_broadcast_bridge,
    sse_response_from_rx,
};
use crate::server::models::{
    CancelExecutionResponse, ErrorResponse, OrchestratorSseEventData, SseMessage,
};
use crate::server::state::{AppState, ExecutionKind};

#[derive(Debug)]
struct FinishedExecutionSnapshot {
    finished_at_ms: i64,
    init_payload: Value,
    snapshot_payload: Value,
    terminal_event: &'static str,
    terminal_payload: Value,
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

        return stream_active_execution(execution).await;
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
        return stream_active_execution(execution).await;
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
        return not_found_response("execution not found or already finished");
    };

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
        let db = crate::server::db::DbPool::connect("sqlite::memory:", 1)
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
