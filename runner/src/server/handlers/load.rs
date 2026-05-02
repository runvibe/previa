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
use tokio::task::JoinSet;
use tracing::error;
use uuid::Uuid;

use previa_runner::{
    Pipeline, RuntimeEnvGroup, RuntimeSpec, execute_pipeline_with_runtime_request_gate,
};

use crate::server::errors::{bad_request_message_response, bad_request_response};
use crate::server::load_dispatch::{DispatchClock, DispatchRuntimeState};
use crate::server::load_wave::{
    calculate_tick_ms, local_rps_limit, sample_intensity, timeline_end_ms, validate_load_profile,
};
use crate::server::metrics::{MetricsAccumulator, estimate_results_network_bytes};
use crate::server::middleware::transaction::{extract_transaction_id, with_transaction_header};
use crate::server::models::{ErrorResponse, LoadProfile, LoadTestConfig, LoadTestRequest};
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
    if payload.load.is_none() && payload.config.is_none() {
        return bad_request_message_response("either load or config must be provided");
    }
    if let Some(load) = payload.load.as_ref() {
        if let Err(message) = validate_load_profile(load) {
            return bad_request_message_response(&message);
        }
    }

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
        let mut executions = state_clone.executions.write().await;
        executions.remove(&execution_id);
        drop(tx);
    });

    sse_response(rx)
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
                let runtime = {
                    let mut lock = runtime_sampler.lock().await;
                    lock.snapshot()
                };

                let snapshot = {
                    let mut lock = metrics.lock().await;
                    lock.update(duration, success);
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

fn wave_snapshot(
    load: &LoadProfile,
    elapsed_ms: u64,
    tick_ms: u64,
    in_flight: usize,
    dispatch: &DispatchRuntimeState,
    missed_starts: usize,
    outstanding_requests: usize,
) -> crate::server::metrics::WaveMetricsSnapshot {
    crate::server::metrics::WaveMetricsSnapshot {
        target_intensity: sample_intensity(load, elapsed_ms),
        target_rps_limit: local_rps_limit(load, elapsed_ms),
        in_flight,
        runner_max_rps: load.runner_max_rps,
        tick_ms,
        scheduled_starts: dispatch.scheduled_total(),
        missed_starts,
        ready_requests: dispatch.waiting_ready_requests(),
        active_pipelines: in_flight,
        outstanding_requests,
    }
}

fn saturating_fetch_sub(value: &AtomicUsize, amount: usize) {
    let mut current = value.load(Ordering::SeqCst);
    loop {
        let next = current.saturating_sub(amount);
        match value.compare_exchange(current, next, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => break,
            Err(observed) => current = observed,
        }
    }
}

async fn run_wave_load(
    load: LoadProfile,
    pipeline: Pipeline,
    selected_key: Option<String>,
    selected_env_group_slug: Option<String>,
    specs: Vec<RuntimeSpec>,
    env_groups: Vec<RuntimeEnvGroup>,
    tx: mpsc::UnboundedSender<SseMessage>,
    token: tokio_util::sync::CancellationToken,
) {
    let tick_ms = calculate_tick_ms(&load);
    let started = Instant::now();
    let end_ms = timeline_end_ms(&load);
    let metrics = Arc::new(tokio::sync::Mutex::new(MetricsAccumulator::new()));
    let runtime_sampler = Arc::new(tokio::sync::Mutex::new(RuntimeSampler::new()));
    let in_flight = Arc::new(AtomicUsize::new(0));
    let dispatch = Arc::new(DispatchRuntimeState::new());
    let missed_starts = Arc::new(AtomicUsize::new(0));
    let outstanding_requests = Arc::new(AtomicUsize::new(0));
    let mut dispatch_clock = DispatchClock::new(tick_ms);
    let mut tasks = JoinSet::new();

    loop {
        while let Some(result) = try_join_finished(&mut tasks).await {
            if let Err(err) = result {
                error!("wave worker join error: {}", err);
            }
        }

        if token.is_cancelled() {
            break;
        }
        let elapsed_ms = started.elapsed().as_millis() as u64;
        if elapsed_ms >= end_ms {
            break;
        }

        let target_rps_limit = local_rps_limit(&load, elapsed_ms);
        let tick = dispatch_clock.plan_tick(elapsed_ms, target_rps_limit);
        dispatch.open_tick(tick);

        let desired_ready = ((target_rps_limit * tick_ms as f64 / 1000.0).ceil() as usize)
            .saturating_mul(2)
            .max(1);
        let spawn_needed = desired_ready.saturating_sub(dispatch.waiting_ready_requests());
        let desired_active = in_flight
            .load(Ordering::SeqCst)
            .saturating_add(spawn_needed)
            .min(load.max_in_flight);

        while in_flight.load(Ordering::SeqCst) < desired_active {
            let pipeline = pipeline.clone();
            let selected_key = selected_key.clone();
            let selected_env_group_slug = selected_env_group_slug.clone();
            let specs = specs.clone();
            let env_groups = env_groups.clone();
            let tx = tx.clone();
            let token = token.clone();
            let metrics = Arc::clone(&metrics);
            let runtime_sampler = Arc::clone(&runtime_sampler);
            let in_flight = Arc::clone(&in_flight);
            let dispatch = Arc::clone(&dispatch);
            let missed_starts = Arc::clone(&missed_starts);
            let outstanding_requests = Arc::clone(&outstanding_requests);
            let load_for_snapshot = load.clone();

            in_flight.fetch_add(1, Ordering::SeqCst);
            {
                let mut lock = metrics.lock().await;
                lock.record_start();
            }
            tasks.spawn(async move {
                let start = Instant::now();
                let metrics_for_gate = Arc::clone(&metrics);
                let token_for_gate = token.clone();
                let dispatch_for_gate = Arc::clone(&dispatch);
                let outstanding_for_gate = Arc::clone(&outstanding_requests);
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
                        let dispatch = Arc::clone(&dispatch_for_gate);
                        let metrics = Arc::clone(&metrics_for_gate);
                        let token = token_for_gate.clone();
                        let outstanding_requests = Arc::clone(&outstanding_for_gate);
                        Box::pin(async move {
                            if dispatch.acquire(|| token.is_cancelled()).await {
                                outstanding_requests.fetch_add(1, Ordering::SeqCst);
                                let mut lock = metrics.lock().await;
                                lock.record_http_start();
                                true
                            } else {
                                false
                            }
                        })
                    },
                )
                .await;
                let duration_ms = start.elapsed().as_millis() as u64;
                let has_error = results.iter().any(|result| result.status == "error");
                let completed_pipeline = results.len() == pipeline.steps.len();
                let success = completed_pipeline && !has_error;
                let (network_tx_bytes, network_rx_bytes) = estimate_results_network_bytes(&results);
                let runtime = {
                    let mut lock = runtime_sampler.lock().await;
                    lock.snapshot()
                };
                let completed_http = results
                    .iter()
                    .filter(|result| result.request.is_some())
                    .count();
                saturating_fetch_sub(&outstanding_requests, completed_http);
                let snapshot = {
                    let mut lock = metrics.lock().await;
                    if completed_pipeline || has_error {
                        lock.update(duration_ms as f64, success);
                    }
                    lock.record_http_completed_count(completed_http);
                    lock.add_network_bytes(network_tx_bytes, network_rx_bytes);
                    let elapsed_ms = started.elapsed().as_millis() as u64;
                    lock.snapshot_with_wave(
                        Some(duration_ms),
                        runtime,
                        Some(wave_snapshot(
                            &load_for_snapshot,
                            elapsed_ms,
                            tick_ms,
                            in_flight.load(Ordering::SeqCst),
                            &dispatch,
                            missed_starts.load(Ordering::SeqCst),
                            outstanding_requests.load(Ordering::SeqCst),
                        )),
                    )
                };
                in_flight.fetch_sub(1, Ordering::SeqCst);
                let _ = send_sse_or_cancel(
                    &tx,
                    "metrics",
                    serde_json::to_value(snapshot).unwrap_or(Value::Null),
                    &token,
                );
            });
        }

        let runtime = {
            let mut lock = runtime_sampler.lock().await;
            lock.snapshot()
        };
        let snapshot = {
            let lock = metrics.lock().await;
            lock.snapshot_with_wave(
                None,
                runtime,
                Some(wave_snapshot(
                    &load,
                    elapsed_ms,
                    tick_ms,
                    in_flight.load(Ordering::SeqCst),
                    &dispatch,
                    missed_starts.load(Ordering::SeqCst),
                    outstanding_requests.load(Ordering::SeqCst),
                )),
            )
        };
        let _ = send_sse_or_cancel(
            &tx,
            "metrics",
            serde_json::to_value(snapshot).unwrap_or(Value::Null),
            &token,
        );

        tokio::time::sleep(tokio::time::Duration::from_millis(tick_ms)).await;
        let report = dispatch.finish_tick();
        missed_starts.fetch_add(report.missed_starts, Ordering::SeqCst);
    }
    dispatch.close();

    let grace_deadline =
        tokio::time::Instant::now() + tokio::time::Duration::from_millis(load.grace_period_ms);
    while !tasks.is_empty() {
        let remaining = grace_deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, tasks.join_next()).await {
            Ok(Some(Err(err))) => error!("wave worker join error: {}", err),
            Ok(Some(Ok(()))) => {}
            Ok(None) => break,
            Err(_) => break,
        }
    }

    if !tasks.is_empty() {
        tasks.abort_all();
    }

    let complete = {
        let lock = metrics.lock().await;
        let runtime = {
            let mut sampler = runtime_sampler.lock().await;
            sampler.snapshot()
        };
        lock.snapshot_with_wave(
            None,
            runtime,
            Some(wave_snapshot(
                &load,
                end_ms,
                tick_ms,
                in_flight.load(Ordering::SeqCst),
                &dispatch,
                missed_starts.load(Ordering::SeqCst),
                outstanding_requests.load(Ordering::SeqCst),
            )),
        )
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

async fn try_join_finished(tasks: &mut JoinSet<()>) -> Option<Result<(), tokio::task::JoinError>> {
    match tokio::time::timeout(tokio::time::Duration::from_millis(0), tasks.join_next()).await {
        Ok(result) => result,
        Err(_) => None,
    }
}
