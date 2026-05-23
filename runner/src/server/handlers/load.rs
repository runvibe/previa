use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tracing::error;
use uuid::Uuid;

use previa_runner::{
    Pipeline, RuntimeEnvGroup, RuntimeSpec, StepExecutionResult,
    execute_pipeline_with_runtime_request_gate,
};

use crate::server::errors::{
    bad_request_message_response, bad_request_response, forbidden_message_response,
};
use crate::server::load_execution::LoadExecutionStatus;
use crate::server::load_wave::validate_load_profile;
use crate::server::metrics::{MetricsAccumulator, estimate_results_network_bytes};
use crate::server::middleware::transaction::{extract_transaction_id, with_transaction_header};
use crate::server::models::{
    ErrorResponse, LoadStartResponse, LoadTelemetryAckRequest, LoadTelemetryAckResponse,
    LoadTelemetryBucket, LoadTelemetryQuery, LoadTelemetryResponse, LoadTestConfig,
    LoadTestRequest,
};
use crate::server::reservation::ReservationError;
use crate::server::runtime::RuntimeSampler;
use crate::server::sse::{SseMessage, send_sse_or_cancel, sse_response};
use crate::server::state::AppState;
use crate::server::wave_executor::run_wave_load;

#[utoipa::path(
    post,
    path = "/api/v1/tests/load",
    tag = "tests",
    params(
        ("x-transaction-id" = Option<String>, Header, description = "ID de transação para rastreamento; será propagado para os requests da pipeline e ecoado no response")
    ),
    request_body = LoadTestRequest,
    responses(
        (
            status = 200,
            description = "Stream SSE com métricas em tempo real.",
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
pub async fn run_load_test(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Result<Json<LoadTestRequest>, JsonRejection>,
) -> Response {
    let Json(payload) = match payload {
        Ok(payload) => payload,
        Err(rejection) => return bad_request_response(rejection),
    };
    if payload.pipeline.steps.is_empty() {
        return bad_request_message_response("pipeline must contain at least one step");
    }
    if payload.load.is_none() && payload.config.is_none() {
        return bad_request_message_response("either load or config must be provided");
    }
    if let Some(load) = payload.load.as_ref() {
        if let Err(message) = validate_load_profile(load) {
            return bad_request_message_response(&message);
        }
    }
    if let Err(err) = state.reservation.validate_first_execution_headers(&headers) {
        let message = match err {
            ReservationError::MissingHeaders => "reservation headers are required",
            ReservationError::InvalidReservation => "reservation headers are invalid",
            ReservationError::Expired => "reservation expired before first use",
        };
        return forbidden_message_response("reservation_forbidden", message);
    }
    state.reservation.mark_execution_started().await;

    let execution_id = Uuid::new_v4().to_string();
    let token = tokio_util::sync::CancellationToken::new();

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
    let config = payload.config.clone();
    let load = payload.load.clone();
    let state_clone = state.clone();
    let execution_id_clone = execution_id.clone();
    let reservation = state.reservation.clone();

    // Cancel execution as soon as SSE client disconnects.
    // A stop token is used so this watcher can exit on normal completion
    // without keeping an extra sender alive and blocking SSE shutdown.
    let tx_disconnect = tx.clone();
    let token_disconnect = token.clone();
    let disconnect_watcher_stop = tokio_util::sync::CancellationToken::new();
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
            reservation.mark_execution_finished().await;
            let mut executions = state_clone.executions.write().await;
            executions.remove(&execution_id);
            return;
        }

        match (load, config) {
            (Some(load), _) => {
                run_wave_load(
                    load,
                    pipeline,
                    selected_key,
                    selected_env_group_slug,
                    specs,
                    env_groups,
                    tx.clone(),
                    token.clone(),
                )
                .await;
            }
            (None, Some(config)) => {
                run_classic_load(
                    config,
                    pipeline,
                    selected_key,
                    selected_env_group_slug,
                    specs,
                    env_groups,
                    tx.clone(),
                    token.clone(),
                )
                .await;
            }
            (None, None) => {}
        }

        disconnect_watcher_stop_exec.cancel();
        reservation.mark_execution_finished().await;
        let mut executions = state_clone.executions.write().await;
        executions.remove(&execution_id);
        drop(tx);
    });

    sse_response(rx)
}

#[utoipa::path(
    post,
    path = "/api/v1/tests/load/start",
    tag = "tests",
    request_body = LoadTestRequest,
    responses(
        (status = 202, description = "Execucao de carga iniciada em background.", body = LoadStartResponse),
        (status = 400, description = "Request invalido", body = ErrorResponse),
        (status = 403, description = "Reserva invalida", body = ErrorResponse)
    )
)]
pub async fn start_load_test(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Result<Json<LoadTestRequest>, JsonRejection>,
) -> Response {
    let Json(payload) = match payload {
        Ok(payload) => payload,
        Err(rejection) => return bad_request_response(rejection),
    };
    if let Err(response) = validate_load_payload(&state, &headers, &payload).await {
        return response;
    }
    state.reservation.mark_execution_started().await;

    let execution_id = Uuid::new_v4().to_string();
    let token = tokio_util::sync::CancellationToken::new();

    {
        let mut executions = state.executions.write().await;
        executions.insert(execution_id.clone(), token.clone());
    }

    let started = state
        .load_executions
        .start(execution_id.clone(), token.clone())
        .await;
    let (tx, mut rx) = mpsc::unbounded_channel::<SseMessage>();
    let store_for_collector = state.load_executions.clone();
    let execution_id_for_collector = execution_id.clone();
    tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            let _ = store_for_collector
                .push_event(&execution_id_for_collector, message.event, message.data)
                .await;
        }
    });

    let selected_key = payload.selected_base_url_key.clone();
    let selected_env_group_slug = payload.selected_env_group_slug.clone();
    let specs = payload.specs.clone();
    let env_groups = payload.env_groups.clone();
    let transaction_id = extract_transaction_id(&headers);
    let pipeline = with_transaction_header(payload.pipeline, transaction_id.as_deref());
    let config = payload.config.clone();
    let load = payload.load.clone();
    let state_clone = state.clone();
    let execution_id_for_task = execution_id.clone();
    let reservation = state.reservation.clone();

    tokio::spawn(async move {
        let _ = send_sse_or_cancel(
            &tx,
            "execution:init",
            json!({ "executionId": execution_id_for_task }),
            &token,
        );

        match (load, config) {
            (Some(load), _) => {
                run_wave_load(
                    load,
                    pipeline,
                    selected_key,
                    selected_env_group_slug,
                    specs,
                    env_groups,
                    tx.clone(),
                    token.clone(),
                )
                .await;
            }
            (None, Some(config)) => {
                run_classic_load(
                    config,
                    pipeline,
                    selected_key,
                    selected_env_group_slug,
                    specs,
                    env_groups,
                    tx.clone(),
                    token.clone(),
                )
                .await;
            }
            (None, None) => {}
        }

        let final_status = if token.is_cancelled() {
            LoadExecutionStatus::Cancelled
        } else {
            LoadExecutionStatus::Completed
        };
        state_clone
            .load_executions
            .finish(&execution_id, final_status)
            .await;
        reservation.mark_execution_finished().await;
        let mut executions = state_clone.executions.write().await;
        executions.remove(&execution_id);
        drop(tx);
    });

    (
        StatusCode::ACCEPTED,
        Json(LoadStartResponse {
            runner_execution_id: started.execution_id,
            status: started.status.as_str().to_owned(),
            next_seq: started.next_seq,
            started_at_ms: started.started_at_ms,
        }),
    )
        .into_response()
}

#[utoipa::path(
    get,
    path = "/api/v1/tests/load/{execution_id}/telemetry",
    tag = "tests",
    params(
        ("execution_id" = String, Path, description = "ID da execucao no runner"),
        ("afterSeq" = Option<u64>, Query, description = "Ultima sequencia ja conhecida pelo main"),
        ("limit" = Option<usize>, Query, description = "Quantidade maxima de buckets retornados")
    ),
    responses(
        (status = 200, description = "Buckets de telemetria ainda nao confirmados.", body = LoadTelemetryResponse),
        (status = 404, description = "Execucao nao encontrada", body = ErrorResponse)
    )
)]
pub async fn get_load_telemetry(
    State(state): State<AppState>,
    Path(execution_id): Path<String>,
    Query(query): Query<LoadTelemetryQuery>,
) -> Response {
    let Some(poll) = state
        .load_executions
        .poll(
            &execution_id,
            query.after_seq.unwrap_or(0),
            query.limit.unwrap_or(256),
        )
        .await
    else {
        return not_found_message_response("execution not found");
    };

    Json(LoadTelemetryResponse {
        runner_execution_id: poll.execution_id,
        status: poll.status.as_str().to_owned(),
        from_seq: poll.from_seq,
        through_seq: poll.through_seq,
        next_seq: poll.next_seq,
        buckets: poll
            .buckets
            .into_iter()
            .map(|bucket| LoadTelemetryBucket {
                seq: bucket.seq,
                event: bucket.event,
                elapsed_ms: bucket.elapsed_ms,
                payload: bucket.payload,
            })
            .collect(),
    })
    .into_response()
}

#[utoipa::path(
    post,
    path = "/api/v1/tests/load/{execution_id}/telemetry/ack",
    tag = "tests",
    params(
        ("execution_id" = String, Path, description = "ID da execucao no runner")
    ),
    request_body = LoadTelemetryAckRequest,
    responses(
        (status = 200, description = "Telemetria confirmada e removida do buffer.", body = LoadTelemetryAckResponse),
        (status = 400, description = "Request invalido", body = ErrorResponse),
        (status = 404, description = "Execucao nao encontrada", body = ErrorResponse)
    )
)]
pub async fn ack_load_telemetry(
    State(state): State<AppState>,
    Path(execution_id): Path<String>,
    payload: Result<Json<LoadTelemetryAckRequest>, JsonRejection>,
) -> Response {
    let Json(payload) = match payload {
        Ok(payload) => payload,
        Err(rejection) => return bad_request_response(rejection),
    };
    let Some(ack) = state
        .load_executions
        .ack(&execution_id, payload.through_seq)
        .await
    else {
        return not_found_message_response("execution not found");
    };

    Json(LoadTelemetryAckResponse {
        runner_execution_id: ack.execution_id,
        acked_through_seq: ack.acked_through_seq,
        retained_from_seq: ack.retained_from_seq,
    })
    .into_response()
}

#[utoipa::path(
    get,
    path = "/api/v1/tests/load/{execution_id}/status",
    tag = "tests",
    params(
        ("execution_id" = String, Path, description = "ID da execucao no runner")
    ),
    responses(
        (status = 200, description = "Status da execucao no runner.", body = serde_json::Value),
        (status = 404, description = "Execucao nao encontrada", body = ErrorResponse)
    )
)]
pub async fn get_load_status(
    State(state): State<AppState>,
    Path(execution_id): Path<String>,
) -> Response {
    let Some(poll) = state.load_executions.poll(&execution_id, 0, 1).await else {
        return not_found_message_response("execution not found");
    };
    Json(json!({
        "runnerExecutionId": poll.execution_id,
        "status": poll.status.as_str(),
        "terminal": poll.status.is_terminal(),
        "nextSeq": poll.next_seq,
        "throughSeq": poll.through_seq
    }))
    .into_response()
}

#[utoipa::path(
    post,
    path = "/api/v1/tests/load/{execution_id}/cancel",
    tag = "tests",
    params(
        ("execution_id" = String, Path, description = "ID da execucao no runner")
    ),
    responses(
        (status = 200, description = "Cancelamento solicitado.", body = serde_json::Value),
        (status = 404, description = "Execucao nao encontrada", body = ErrorResponse)
    )
)]
pub async fn cancel_load_test(
    State(state): State<AppState>,
    Path(execution_id): Path<String>,
) -> Response {
    if !state.load_executions.cancel(&execution_id).await {
        return not_found_message_response("execution not found");
    }
    state
        .load_executions
        .finish(&execution_id, LoadExecutionStatus::Cancelled)
        .await;
    Json(json!({ "runnerExecutionId": execution_id, "status": "cancelled" })).into_response()
}

async fn validate_load_payload(
    state: &AppState,
    headers: &HeaderMap,
    payload: &LoadTestRequest,
) -> Result<(), Response> {
    if payload.pipeline.steps.is_empty() {
        return Err(bad_request_message_response(
            "pipeline must contain at least one step",
        ));
    }
    if payload.load.is_none() && payload.config.is_none() {
        return Err(bad_request_message_response(
            "either load or config must be provided",
        ));
    }
    if let Some(load) = payload.load.as_ref() {
        if let Err(message) = validate_load_profile(load) {
            return Err(bad_request_message_response(&message));
        }
    }
    if let Err(err) = state.reservation.validate_first_execution_headers(headers) {
        let message = match err {
            ReservationError::MissingHeaders => "reservation headers are required",
            ReservationError::InvalidReservation => "reservation headers are invalid",
            ReservationError::Expired => "reservation expired before first use",
        };
        return Err(forbidden_message_response("reservation_forbidden", message));
    }
    Ok(())
}

fn not_found_message_response(message: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: "not_found".to_owned(),
            message: message.to_owned(),
        }),
    )
        .into_response()
}

async fn run_classic_load(
    config: LoadTestConfig,
    pipeline: Pipeline,
    selected_key: Option<String>,
    selected_env_group_slug: Option<String>,
    specs: Vec<RuntimeSpec>,
    env_groups: Vec<RuntimeEnvGroup>,
    tx: mpsc::UnboundedSender<SseMessage>,
    token: tokio_util::sync::CancellationToken,
) {
    let total_requests = config.total_requests.max(1);
    let concurrency = config.concurrency.max(1).min(total_requests);
    let ramp_interval_ms = if concurrency > 1 && config.ramp_up_seconds > 0.0 {
        ((config.ramp_up_seconds * 1000.0) / ((concurrency - 1) as f64)).round() as u64
    } else {
        0
    };

    let counter = Arc::new(AtomicUsize::new(0));
    let metrics = Arc::new(tokio::sync::Mutex::new(MetricsAccumulator::new()));
    let runtime_sampler = Arc::new(tokio::sync::Mutex::new(RuntimeSampler::new()));

    let mut handles = Vec::with_capacity(concurrency);

    for worker_idx in 0..concurrency {
        if token.is_cancelled() {
            break;
        }

        if worker_idx > 0 && ramp_interval_ms > 0 {
            tokio::time::sleep(tokio::time::Duration::from_millis(ramp_interval_ms)).await;
        }

        let counter = Arc::clone(&counter);
        let metrics = Arc::clone(&metrics);
        let runtime_sampler = Arc::clone(&runtime_sampler);
        let tx = tx.clone();
        let token = token.clone();
        let pipeline = pipeline.clone();
        let selected_key = selected_key.clone();
        let selected_env_group_slug = selected_env_group_slug.clone();
        let specs = specs.clone();
        let env_groups = env_groups.clone();

        handles.push(tokio::spawn(async move {
            loop {
                if token.is_cancelled() {
                    break;
                }

                let idx = counter.fetch_add(1, Ordering::SeqCst);
                if idx >= total_requests {
                    break;
                }

                {
                    let mut lock = metrics.lock().await;
                    lock.record_start();
                }

                let start = Instant::now();
                let metrics_for_gate = Arc::clone(&metrics);
                let results = execute_pipeline_with_runtime_request_gate(
                    &pipeline,
                    selected_key.as_deref(),
                    Some(specs.as_slice()),
                    Some(env_groups.as_slice()),
                    selected_env_group_slug.as_deref(),
                    |_| {},
                    |_| {},
                    || token.is_cancelled(),
                    move |_| {
                        let metrics = Arc::clone(&metrics_for_gate);
                        Box::pin(async move {
                            let mut lock = metrics.lock().await;
                            lock.record_http_start();
                            true
                        })
                    },
                )
                .await;
                let duration_ms = start.elapsed().as_millis() as u64;
                let duration = duration_ms as f64;
                let success = !results.iter().any(|r| r.status == "error");
                let (network_tx_bytes, network_rx_bytes) = estimate_results_network_bytes(&results);
                let terminal_http_status = terminal_http_status(&results, success);
                let runtime = {
                    let mut lock = runtime_sampler.lock().await;
                    lock.snapshot()
                };

                let snapshot = {
                    let mut lock = metrics.lock().await;
                    lock.update(duration, success);
                    if terminal_http_status.is_some() || !success {
                        lock.record_status_code(terminal_http_status);
                    }
                    lock.record_http_completed_count(
                        results
                            .iter()
                            .filter(|result| result.request.is_some())
                            .count(),
                    );
                    lock.add_network_bytes(network_tx_bytes, network_rx_bytes);
                    lock.snapshot(Some(duration_ms), runtime)
                };

                if !send_sse_or_cancel(
                    &tx,
                    "metrics",
                    serde_json::to_value(snapshot).unwrap_or(Value::Null),
                    &token,
                ) {
                    break;
                }
            }
        }));
    }

    for handle in handles {
        if let Err(err) = handle.await {
            error!("worker join error: {}", err);
        }
    }

    let complete = {
        let lock = metrics.lock().await;
        let runtime = {
            let mut sampler = runtime_sampler.lock().await;
            sampler.snapshot()
        };
        lock.snapshot(None, runtime)
    };

    if !token.is_cancelled() {
        let _ = send_sse_or_cancel(
            &tx,
            "complete",
            serde_json::to_value(complete).unwrap_or(Value::Null),
            &token,
        );
    }
}

fn terminal_http_status(results: &[StepExecutionResult], success: bool) -> Option<u16> {
    if !success {
        return results
            .iter()
            .find(|result| result.status == "error")
            .and_then(|result| result.response.as_ref().map(|response| response.status))
            .or_else(|| {
                results
                    .iter()
                    .rev()
                    .find_map(|result| result.response.as_ref().map(|response| response.status))
            });
    }

    results
        .iter()
        .rev()
        .find_map(|result| result.response.as_ref().map(|response| response.status))
}

#[cfg(test)]
mod polling_tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use crate::server::build_app;
    use crate::server::state::AppState;

    fn load_request() -> Value {
        json!({
            "pipeline": {
                "id": "pipe-test",
                "name": "Polling pipeline",
                "description": null,
                "steps": [
                    {
                        "id": "step-1",
                        "name": "GET unavailable",
                        "description": null,
                        "method": "GET",
                        "url": "http://127.0.0.1:1",
                        "headers": {},
                        "body": null,
                        "asserts": []
                    }
                ]
            },
            "config": {
                "totalRequests": 1,
                "concurrency": 1,
                "rampUpSeconds": 0
            },
            "selectedBaseUrlKey": null,
            "selectedEnvGroupSlug": null,
            "specs": [],
            "envGroups": []
        })
    }

    async fn json_response(response: axum::response::Response) -> Value {
        let body = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn load_start_runs_in_background_and_exposes_ackable_polling_telemetry() {
        let app = build_app(AppState::default());
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/tests/load/start")
                    .header("content-type", "application/json")
                    .body(Body::from(load_request().to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let started = json_response(response).await;
        let execution_id = started["runnerExecutionId"]
            .as_str()
            .expect("runnerExecutionId")
            .to_owned();
        assert_eq!(started["status"], "running");

        let mut telemetry = Value::Null;
        for _ in 0..20 {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("GET")
                        .uri(format!(
                            "/api/v1/tests/load/{}/telemetry?afterSeq=0&limit=10",
                            execution_id
                        ))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            telemetry = json_response(response).await;
            if telemetry["throughSeq"].as_u64().unwrap_or(0) > 0 {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(25)).await;
        }

        let through_seq = telemetry["throughSeq"]
            .as_u64()
            .expect("throughSeq should be present");
        assert!(through_seq > 0);
        assert!(!telemetry["buckets"].as_array().unwrap().is_empty());

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/tests/load/{}/telemetry/ack", execution_id))
                    .header("content-type", "application/json")
                    .body(Body::from(json!({ "throughSeq": through_seq }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let ack = json_response(response).await;
        assert_eq!(ack["ackedThroughSeq"], through_seq);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!(
                        "/api/v1/tests/load/{}/telemetry?afterSeq=0&limit=10",
                        execution_id
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let after_ack = json_response(response).await;
        assert!(after_ack["buckets"].as_array().unwrap().is_empty());
    }
}

#[cfg(test)]
mod reservation_tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use serde_json::json;
    use tower::ServiceExt;

    use crate::server::build_app;
    use crate::server::reservation::ReservationState;
    use crate::server::state::AppState;

    fn reserved_app() -> Router {
        build_app(AppState {
            reservation: ReservationState::reserved_for_test(
                "rr_test",
                "secret-token",
                "2999-01-01T00:00:00Z",
            ),
            ..AppState::default()
        })
    }

    fn load_request() -> serde_json::Value {
        json!({
            "pipeline": {
                "id": "pipe-test",
                "name": "Reserved pipeline",
                "description": null,
                "steps": [
                    {
                        "id": "step-1",
                        "name": "GET example",
                        "description": null,
                        "method": "GET",
                        "url": "http://127.0.0.1:1",
                        "headers": {},
                        "body": null,
                        "asserts": []
                    }
                ]
            },
            "config": {
                "totalRequests": 1,
                "concurrency": 1,
                "rampUpSeconds": 0
            },
            "selectedBaseUrlKey": null,
            "selectedEnvGroupSlug": null,
            "specs": [],
            "envGroups": []
        })
    }

    #[tokio::test]
    async fn reserved_runner_rejects_load_without_reservation_headers() {
        let response = reserved_app()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/tests/load")
                    .header("content-type", "application/json")
                    .body(Body::from(load_request().to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["error"], "reservation_forbidden");
    }

    #[tokio::test]
    async fn reserved_runner_rejects_load_with_wrong_reservation_token() {
        let response = reserved_app()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/tests/load")
                    .header("content-type", "application/json")
                    .header("x-previa-reservation-id", "rr_test")
                    .header("x-previa-reservation-token", "wrong-token")
                    .body(Body::from(load_request().to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["error"], "reservation_forbidden");
    }

    #[tokio::test]
    async fn reserved_runner_accepts_matching_headers_and_reports_consumed_execution() {
        let app = reserved_app();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/tests/load")
                    .header("content-type", "application/json")
                    .header("x-previa-reservation-id", "rr_test")
                    .header("x-previa-reservation-token", "secret-token")
                    .body(Body::from(load_request().to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let _ = response.into_body().collect().await.unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/info")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["startedExecutionCount"], 1);
        assert_eq!(payload["busy"], false);
        assert!(payload["lastStartedAt"].is_string());
        assert!(payload["lastFinishedAt"].is_string());
    }

    #[tokio::test]
    async fn rearmed_runner_rejects_old_token_and_accepts_new_token() {
        let app = reserved_app();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/tests/load")
                    .header("content-type", "application/json")
                    .header("x-previa-reservation-id", "rr_test")
                    .header("x-previa-reservation-token", "secret-token")
                    .body(Body::from(load_request().to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let _ = response.into_body().collect().await.unwrap();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/internal/reservation/rearm")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "reservationId": "rr_next",
                            "reservationToken": "next-token",
                            "expiresAt": "2999-01-01T00:00:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/tests/load")
                    .header("content-type", "application/json")
                    .header("x-previa-reservation-id", "rr_test")
                    .header("x-previa-reservation-token", "secret-token")
                    .body(Body::from(load_request().to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/tests/load")
                    .header("content-type", "application/json")
                    .header("x-previa-reservation-id", "rr_next")
                    .header("x-previa-reservation-token", "next-token")
                    .body(Body::from(load_request().to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn busy_runner_rejects_rearm_and_release() {
        let state = AppState {
            reservation: ReservationState::reserved_for_test(
                "rr_test",
                "secret-token",
                "2999-01-01T00:00:00Z",
            ),
            ..AppState::default()
        };
        state.reservation.mark_execution_started().await;
        let app = build_app(state);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/internal/reservation/rearm")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "reservationId": "rr_next",
                            "reservationToken": "next-token",
                            "expiresAt": "2999-01-01T00:00:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/internal/reservation/release")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn release_allows_idle_runner_to_be_rearmed() {
        let app = reserved_app();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/internal/reservation/release")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/internal/reservation/rearm")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "reservationId": "rr_next",
                            "reservationToken": "next-token",
                            "expiresAt": "2999-01-01T00:00:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }
}
