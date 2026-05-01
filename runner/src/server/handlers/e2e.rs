use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::HeaderMap;
use axum::response::Response;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use previa_runner::execute_pipeline_with_runtime_hooks;

use crate::server::errors::{bad_request_message_response, bad_request_response};
use crate::server::middleware::transaction::{extract_transaction_id, with_transaction_header};
use crate::server::models::{E2eSummary, E2eTestRequest, ErrorResponse};
use crate::server::sse::{SseMessage, send_sse_or_cancel, sse_response};
use crate::server::state::AppState;

#[utoipa::path(
    post,
    path = "/api/v1/tests/e2e",
    tag = "tests",
    params(
        ("x-transaction-id" = Option<String>, Header, description = "ID de transação para rastreamento; será propagado para os requests da pipeline e ecoado no response")
    ),
    request_body = E2eTestRequest,
    responses(
        (
            status = 200,
            description = "Stream SSE com eventos de execução",
            content_type = "text/event-stream",
            body = String,
            headers(
                ("x-transaction-id" = Option<String>, description = "Eco do x-transaction-id recebido")
            )
        ),
        (
            status = 400,
            description = "Request inválido",
            body = ErrorResponse,
            headers(
                ("x-transaction-id" = Option<String>, description = "Eco do x-transaction-id recebido")
            )
        )
    )
)]
pub async fn run_e2e_test(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Result<Json<E2eTestRequest>, JsonRejection>,
) -> Response {
    let Json(payload) = match payload {
        Ok(payload) => payload,
        Err(rejection) => return bad_request_response(rejection),
    };
    if payload.pipeline.steps.is_empty() {
        return bad_request_message_response("pipeline must contain at least one step");
    }

    let execution_id = Uuid::new_v4().to_string();
    let token = CancellationToken::new();

    {
        let mut executions = state.executions.write().await;
        executions.insert(execution_id.clone(), token.clone());
    }

    let (tx, rx) = mpsc::unbounded_channel::<SseMessage>();
    let selected_key = payload.selected_base_url_key.clone();
    let selected_env_group_slug = payload.selected_env_group_slug.clone();
    let specs = payload.specs.clone();
    let env_groups = payload.env_groups.clone();
    let transaction_id = extract_transaction_id(&headers);
    let pipeline = with_transaction_header(payload.pipeline, transaction_id.as_deref());
    let state_clone = state.clone();
    let execution_id_clone = execution_id.clone();

    // Cancel execution as soon as SSE client disconnects.
    // A stop token is used so this watcher can exit on normal completion
    // without keeping an extra sender alive and blocking SSE shutdown.
    let tx_disconnect = tx.clone();
    let token_disconnect = token.clone();
    let disconnect_watcher_stop = CancellationToken::new();
    let disconnect_watcher_stop_task = disconnect_watcher_stop.clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = tx_disconnect.closed() => token_disconnect.cancel(),
            _ = disconnect_watcher_stop_task.cancelled() => {}
        }
    });
    let disconnect_watcher_stop_exec = disconnect_watcher_stop.clone();

    tokio::spawn(async move {
        if !send_sse_or_cancel(
            &tx,
            "execution:init",
            json!({ "executionId": execution_id_clone }),
            &token,
        ) {
            disconnect_watcher_stop_exec.cancel();
            let mut executions = state_clone.executions.write().await;
            executions.remove(&execution_id);
            return;
        }

        let results = execute_pipeline_with_runtime_hooks(
            &pipeline,
            selected_key.as_deref(),
            Some(specs.as_slice()),
            Some(env_groups.as_slice()),
            selected_env_group_slug.as_deref(),
            |step_id| {
                let _ = send_sse_or_cancel(&tx, "step:start", json!({ "stepId": step_id }), &token);
            },
            |result| {
                let _ = send_sse_or_cancel(
                    &tx,
                    "step:result",
                    serde_json::to_value(result).unwrap_or(Value::Null),
                    &token,
                );
            },
            || token.is_cancelled(),
        )
        .await;

        let summary = E2eSummary {
            total_steps: results.len(),
            passed: results.iter().filter(|r| r.status == "success").count(),
            failed: results.iter().filter(|r| r.status == "error").count(),
            total_duration: results.iter().map(|r| r.duration.unwrap_or(0)).sum(),
        };

        if !token.is_cancelled() {
            let _ = send_sse_or_cancel(
                &tx,
                "pipeline:complete",
                serde_json::to_value(summary).unwrap_or(Value::Null),
                &token,
            );
        }

        disconnect_watcher_stop_exec.cancel();
        let mut executions = state_clone.executions.write().await;
        executions.remove(&execution_id);
        drop(tx);
    });

    sse_response(rx)
}
