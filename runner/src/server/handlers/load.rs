use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::HeaderMap;
use axum::response::Response;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tracing::error;
use uuid::Uuid;

use previa_runner::execute_pipeline_with_specs_hooks;

use crate::server::errors::{bad_request_message_response, bad_request_response};
use crate::server::metrics::{MetricsAccumulator, estimate_results_network_bytes};
use crate::server::middleware::transaction::{extract_transaction_id, with_transaction_header};
use crate::server::models::{ErrorResponse, LoadTestRequest};
use crate::server::runtime::RuntimeSampler;
use crate::server::sse::{SseMessage, send_sse_or_cancel, sse_response};
use crate::server::state::AppState;

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

    let execution_id = Uuid::new_v4().to_string();
    let token = tokio_util::sync::CancellationToken::new();

    {
        let mut executions = state.executions.write().await;
        executions.insert(execution_id.clone(), token.clone());
    }

    let (tx, rx) = mpsc::unbounded_channel::<SseMessage>();
    let selected_key = payload.selected_base_url_key.clone();
    let specs = payload.specs.clone();
    let transaction_id = extract_transaction_id(&headers);
    let pipeline = with_transaction_header(payload.pipeline, transaction_id.as_deref());
    let config = payload.config;
    let state_clone = state.clone();
    let execution_id_clone = execution_id.clone();

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
            let mut executions = state_clone.executions.write().await;
            executions.remove(&execution_id);
            return;
        }

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
            let specs = specs.clone();

            handles.push(tokio::spawn(async move {
                loop {
                    if token.is_cancelled() {
                        break;
                    }

                    let idx = counter.fetch_add(1, Ordering::SeqCst);
                    if idx >= total_requests {
                        break;
                    }

                    let start = Instant::now();
                    let results = execute_pipeline_with_specs_hooks(
                        &pipeline,
                        selected_key.as_deref(),
                        Some(specs.as_slice()),
                        |_| {},
                        |_| {},
                        || token.is_cancelled(),
                    )
                    .await;
                    let duration_ms = start.elapsed().as_millis() as u64;
                    let duration = duration_ms as f64;
                    let success = !results.iter().any(|r| r.status == "error");
                    let (network_tx_bytes, network_rx_bytes) =
                        estimate_results_network_bytes(&results);
                    let runtime = {
                        let mut lock = runtime_sampler.lock().await;
                        lock.snapshot()
                    };

                    let snapshot = {
                        let mut lock = metrics.lock().await;
                        lock.update(duration, success);
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

        disconnect_watcher_stop_exec.cancel();
        let mut executions = state_clone.executions.write().await;
        executions.remove(&execution_id);
        drop(tx);
    });

    sse_response(rx)
}
