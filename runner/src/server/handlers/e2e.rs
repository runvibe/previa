use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::HeaderMap;
use axum::response::Response;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use previa_runner::{
    execute_pipeline_from_step_with_client_runtime_hooks, execute_pipeline_with_runtime_hooks,
};

use crate::server::errors::{bad_request_message_response, bad_request_response};
use crate::server::middleware::transaction::{extract_transaction_id, with_transaction_header};
use crate::server::models::{E2eRerunFromStepRequest, E2eSummary, E2eTestRequest, ErrorResponse};
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

#[utoipa::path(
    post,
    path = "/api/v1/tests/e2e/rerun-from-step",
    tag = "tests",
    params(
        ("x-transaction-id" = Option<String>, Header, description = "ID de transação para rastreamento; será propagado para os requests da pipeline e ecoado no response")
    ),
    request_body = E2eRerunFromStepRequest,
    responses(
        (
            status = 200,
            description = "Stream SSE com eventos de reexecução parcial",
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
pub async fn rerun_e2e_from_step(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Result<Json<E2eRerunFromStepRequest>, JsonRejection>,
) -> Response {
    let Json(payload) = match payload {
        Ok(payload) => payload,
        Err(rejection) => return bad_request_response(rejection),
    };
    if payload.pipeline.steps.is_empty() {
        return bad_request_message_response("pipeline must contain at least one step");
    }
    if !payload
        .pipeline
        .steps
        .iter()
        .any(|step| step.id == payload.start_step_id)
    {
        return bad_request_message_response("startStepId not found in pipeline");
    }

    let execution_id = Uuid::new_v4().to_string();
    let token = CancellationToken::new();

    {
        let mut executions = state.executions.write().await;
        executions.insert(execution_id.clone(), token.clone());
    }

    let (tx, rx) = mpsc::unbounded_channel::<SseMessage>();
    let selected_env_group_slug = payload.selected_env_group_slug.clone();
    let specs = payload.specs.clone();
    let env_groups = payload.env_groups.clone();
    let start_step_id = payload.start_step_id.clone();
    let prior_results = payload.prior_results.clone();
    let transaction_id = extract_transaction_id(&headers);
    let pipeline = with_transaction_header(payload.pipeline, transaction_id.as_deref());
    let state_clone = state.clone();
    let execution_id_clone = execution_id.clone();

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

        let client = reqwest::Client::new();
        let results = execute_pipeline_from_step_with_client_runtime_hooks(
            &client,
            &pipeline,
            &start_step_id,
            prior_results,
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
            |_| Box::pin(async { true }),
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

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request, StatusCode, header};
    use httpmock::Method::GET;
    use httpmock::MockServer;
    use serde_json::json;
    use tower::ServiceExt;

    use crate::server::build_app;
    use crate::server::state::AppState;

    #[tokio::test]
    async fn rerun_from_step_streams_only_suffix_steps_with_prior_context() {
        let target = MockServer::start_async().await;
        let protected = target
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/protected")
                    .header("authorization", "Bearer abc123");
                then.status(200)
                    .header("content-type", "application/json")
                    .json_body(json!({ "ok": true }));
            })
            .await;

        let app = build_app(AppState::default());
        let payload = json!({
            "pipeline": {
                "id": "pipe-1",
                "name": "Pipe",
                "description": null,
                "steps": [
                    {
                        "id": "login",
                        "name": "Login",
                        "description": null,
                        "method": "POST",
                        "url": format!("{}/login", target.base_url()),
                        "headers": {},
                        "body": null,
                        "asserts": []
                    },
                    {
                        "id": "protected",
                        "name": "Protected",
                        "description": null,
                        "method": "GET",
                        "url": format!("{}/protected", target.base_url()),
                        "headers": { "Authorization": "Bearer {{steps.login.token}}" },
                        "body": null,
                        "asserts": []
                    }
                ]
            },
            "startStepId": "protected",
            "priorResults": {
                "login": {
                    "stepId": "login",
                    "status": "success",
                    "request": {
                        "method": "POST",
                        "url": format!("{}/login", target.base_url()),
                        "headers": {},
                        "body": null
                    },
                    "response": {
                        "status": 200,
                        "statusText": "OK",
                        "headers": {},
                        "body": { "token": "abc123" }
                    },
                    "duration": 1,
                    "attempt": 1,
                    "maxAttempts": 1
                }
            }
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/tests/e2e/rerun-from-step")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(!text.contains("\"stepId\":\"login\""));
        assert!(text.contains("event: step:start"));
        assert!(text.contains("\"stepId\":\"protected\""));
        assert!(text.contains("event: pipeline:complete"));
        protected.assert_async().await;
    }
}
