use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::sync::{Mutex, Semaphore, broadcast};
use tokio_stream::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::server::execution::forward::{parse_sse_block, send_sse_best_effort};
use crate::server::execution::history_capture::{extract_error_message, push_load_error};
use crate::server::execution::runner_auth::apply_runner_auth;
use crate::server::execution::scheduler::SharedValue;
use crate::server::execution::snapshot::build_live_load_snapshot_payload;
use crate::server::models::{
    ConsolidatedLoadLifecycleBucket, ConsolidatedLoadMetrics, ConsolidatedLoadStatusCodeBucket,
    LoadEventContext, LoadLatencyAccumulator, LoadLatencySummary, RunnerLoadDispatchBucket,
    RunnerLoadLifecycleBucket, RunnerLoadLine, RunnerLoadSnapshotMode, RunnerLoadStatusCodeBucket,
};
use crate::server::state::TRANSACTION_ID_HEADER;
use crate::server::utils::{now_ms, parse_runner_duration_ms, parse_runner_load_metrics};

const RPS_HISTORY_BUCKET_MS: u64 = 1_000;
const RPS_HISTORY_CORRECTION_WINDOW_MS: u64 = 10_000;
const RUNNER_LOAD_POLL_INTERVAL_MS: u64 = 1_000;
const RUNNER_LOAD_POLL_LIMIT: usize = 512;
const RUNNER_LOAD_POLL_CONCURRENCY: usize = 100;

pub fn runner_load_poll_concurrency() -> usize {
    std::env::var("PREVIA_RUNNER_LOAD_POLL_CONCURRENCY")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(RUNNER_LOAD_POLL_CONCURRENCY)
        .max(1)
}

#[derive(Debug, Clone, Default)]
pub struct LoadTelemetryState {
    runners: HashMap<String, RunnerLoadTelemetryState>,
}

#[derive(Debug, Clone)]
pub struct RunnerReservationHeaders {
    pub reservation_id: String,
    pub reservation_token: String,
}

#[derive(Debug, Clone, Default)]
struct RunnerLoadTelemetryState {
    line: Option<RunnerLoadLine>,
    dispatch_buckets: BTreeMap<u64, RunnerLoadDispatchBucket>,
    lifecycle_buckets: BTreeMap<u64, RunnerLoadLifecycleBucket>,
    status_code_buckets: BTreeMap<(u64, String), RunnerLoadStatusCodeBucket>,
}

#[allow(dead_code)]
pub async fn forward_runner_stream_load_chunked(
    client: &Client,
    node: String,
    body: Value,
    tx: broadcast::Sender<crate::server::models::SseMessage>,
    cancel: CancellationToken,
    load_chunk: Arc<Mutex<HashMap<String, RunnerLoadLine>>>,
    load_telemetry: Arc<Mutex<LoadTelemetryState>>,
    load_latency: Arc<Mutex<LoadLatencyAccumulator>>,
    load_errors: Arc<Mutex<Vec<String>>>,
    load_context: Arc<LoadEventContext>,
    execution_id: String,
    snapshot_payload: SharedValue<Value>,
    endpoint_path: &str,
    transaction_id: Option<String>,
    runner_auth_key: Option<&str>,
    reservation_headers: Option<RunnerReservationHeaders>,
) {
    if cancel.is_cancelled() {
        return;
    }

    let url = format!("{}{}", node.trim_end_matches('/'), endpoint_path);

    let mut request = apply_runner_auth(
        client
            .post(url)
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream"),
        runner_auth_key,
    );

    if let Some(transaction_id) = transaction_id.as_deref() {
        request = request.header(TRANSACTION_ID_HEADER, transaction_id);
    }
    if let Some(headers) = reservation_headers {
        request = request
            .header("x-previa-reservation-id", headers.reservation_id)
            .header("x-previa-reservation-token", headers.reservation_token);
    }

    let response = match tokio::time::timeout(Duration::from_secs(10), request.json(&body).send())
        .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(err)) => {
            push_load_error(&load_errors, format!("runner request failed: {}", err)).await;
            refresh_load_snapshot_from_telemetry(
                &execution_id,
                &snapshot_payload,
                &load_telemetry,
                &load_latency,
                &load_errors,
                load_context.as_ref(),
                "running",
            )
            .await;
            let payload = add_load_context_fields(
                json!({
                    "message": format!("runner request failed: {}", err),
                    "lines": [RunnerLoadLine {
                        node: node.clone(),
                        runner_event: "error".to_owned(),
                        received_at: now_ms(),
                        payload: json!({ "message": format!("runner request failed: {}", err) }),
                    }]
                }),
                load_context.as_ref(),
            );
            let _ = send_sse_best_effort(&tx, "error", payload);
            return;
        }
        Err(_) => {
            push_load_error(&load_errors, "runner request timeout".to_owned()).await;
            refresh_load_snapshot_from_telemetry(
                &execution_id,
                &snapshot_payload,
                &load_telemetry,
                &load_latency,
                &load_errors,
                load_context.as_ref(),
                "running",
            )
            .await;
            let payload = add_load_context_fields(
                json!({
                    "message": "runner request timeout",
                    "lines": [RunnerLoadLine {
                        node: node.clone(),
                        runner_event: "error".to_owned(),
                        received_at: now_ms(),
                        payload: json!({ "message": "runner request timeout" }),
                    }]
                }),
                load_context.as_ref(),
            );
            let _ = send_sse_best_effort(&tx, "error", payload);
            return;
        }
    };

    if !response.status().is_success() {
        let status = response.status().as_u16();
        let body_text = response.text().await.unwrap_or_default();
        let message = format!("runner returned HTTP {}: {}", status, body_text);
        push_load_error(&load_errors, message.clone()).await;
        refresh_load_snapshot_from_telemetry(
            &execution_id,
            &snapshot_payload,
            &load_telemetry,
            &load_latency,
            &load_errors,
            load_context.as_ref(),
            "running",
        )
        .await;
        let payload = add_load_context_fields(
            json!({
                "message": message,
                "lines": [RunnerLoadLine {
                    node: node.clone(),
                    runner_event: "error".to_owned(),
                    received_at: now_ms(),
                    payload: json!({ "message": format!("runner returned HTTP {}: {}", status, body_text) }),
                }]
            }),
            load_context.as_ref(),
        );
        let _ = send_sse_best_effort(&tx, "error", payload);
        return;
    }

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    loop {
        let next_chunk = tokio::select! {
            _ = cancel.cancelled() => {
                return;
            }
            chunk = stream.next() => chunk,
        };

        let Some(chunk_result) = next_chunk else {
            break;
        };

        let chunk = match chunk_result {
            Ok(chunk) => chunk,
            Err(err) => {
                push_load_error(&load_errors, format!("runner stream read error: {}", err)).await;
                refresh_load_snapshot_from_telemetry(
                    &execution_id,
                    &snapshot_payload,
                    &load_telemetry,
                    &load_latency,
                    &load_errors,
                    load_context.as_ref(),
                    "running",
                )
                .await;
                let payload = add_load_context_fields(
                    json!({
                        "message": format!("runner stream read error: {}", err),
                        "lines": [RunnerLoadLine {
                            node: node.clone(),
                            runner_event: "error".to_owned(),
                            received_at: now_ms(),
                            payload: json!({ "message": format!("runner stream read error: {}", err) }),
                        }]
                    }),
                    load_context.as_ref(),
                );
                let _ = send_sse_best_effort(&tx, "error", payload);
                return;
            }
        };

        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(idx) = buffer.find("\n\n") {
            let block = buffer[..idx].to_owned();
            buffer = buffer[idx + 2..].to_owned();

            let Some((event, data_text)) = parse_sse_block(&block) else {
                continue;
            };

            if event == "execution:init" {
                continue;
            }

            let data = serde_json::from_str::<Value>(&data_text)
                .unwrap_or_else(|_| Value::String(data_text.clone()));
            if event == "error" {
                push_load_error(&load_errors, extract_error_message(&data)).await;
            }
            if event == "metrics" {
                if let Some(duration_ms) = parse_runner_duration_ms(&data) {
                    let mut lock = load_latency.lock().await;
                    lock.add_sample(duration_ms);
                }
                merge_runner_error_samples(&load_errors, &node, &data).await;
            }
            let line = RunnerLoadLine {
                node: node.clone(),
                runner_event: event,
                received_at: now_ms(),
                payload: data,
            };

            {
                let mut lock = load_chunk.lock().await;
                lock.insert(node.clone(), line.clone());
            }

            let mut telemetry_lock = load_telemetry.lock().await;
            apply_runner_telemetry_line(&mut telemetry_lock, line);
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunnerLoadStartResponse {
    runner_execution_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunnerLoadTelemetryResponse {
    status: String,
    buckets: Vec<RunnerLoadTelemetryBucket>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunnerLoadTelemetryBucket {
    seq: u64,
    event: String,
    payload: Value,
}

pub async fn forward_runner_polled_load_chunked(
    client: &Client,
    node: String,
    body: Value,
    tx: broadcast::Sender<crate::server::models::SseMessage>,
    cancel: CancellationToken,
    load_chunk: Arc<Mutex<HashMap<String, RunnerLoadLine>>>,
    load_telemetry: Arc<Mutex<LoadTelemetryState>>,
    load_latency: Arc<Mutex<LoadLatencyAccumulator>>,
    load_errors: Arc<Mutex<Vec<String>>>,
    load_context: Arc<LoadEventContext>,
    execution_id: String,
    snapshot_payload: SharedValue<Value>,
    endpoint_path: &str,
    transaction_id: Option<String>,
    runner_auth_key: Option<&str>,
    reservation_headers: Option<RunnerReservationHeaders>,
    poll_permits: Arc<Semaphore>,
) {
    if cancel.is_cancelled() {
        return;
    }

    let start_url = format!("{}{}", node.trim_end_matches('/'), endpoint_path);
    let telemetry_base_path = endpoint_path.trim_end_matches("/start");
    let mut request = apply_runner_auth(
        client
            .post(start_url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json"),
        runner_auth_key,
    );

    if let Some(transaction_id) = transaction_id.as_deref() {
        request = request.header(TRANSACTION_ID_HEADER, transaction_id);
    }
    if let Some(headers) = reservation_headers.as_ref() {
        request = request
            .header("x-previa-reservation-id", headers.reservation_id.as_str())
            .header(
                "x-previa-reservation-token",
                headers.reservation_token.as_str(),
            );
    }

    let response =
        match tokio::time::timeout(Duration::from_secs(10), request.json(&body).send()).await {
            Ok(Ok(response)) => response,
            Ok(Err(err)) => {
                emit_runner_load_request_error(
                    &node,
                    &tx,
                    &load_errors,
                    &load_telemetry,
                    &load_latency,
                    &load_context,
                    &execution_id,
                    &snapshot_payload,
                    format!("runner start request failed: {}", err),
                )
                .await;
                return;
            }
            Err(_) => {
                emit_runner_load_request_error(
                    &node,
                    &tx,
                    &load_errors,
                    &load_telemetry,
                    &load_latency,
                    &load_context,
                    &execution_id,
                    &snapshot_payload,
                    "runner start request timeout".to_owned(),
                )
                .await;
                return;
            }
        };

    if !response.status().is_success() {
        let status = response.status().as_u16();
        let body_text = response.text().await.unwrap_or_default();
        emit_runner_load_request_error(
            &node,
            &tx,
            &load_errors,
            &load_telemetry,
            &load_latency,
            &load_context,
            &execution_id,
            &snapshot_payload,
            format!("runner start returned HTTP {}: {}", status, body_text),
        )
        .await;
        return;
    }

    let started = match response.json::<RunnerLoadStartResponse>().await {
        Ok(started) => started,
        Err(err) => {
            emit_runner_load_request_error(
                &node,
                &tx,
                &load_errors,
                &load_telemetry,
                &load_latency,
                &load_context,
                &execution_id,
                &snapshot_payload,
                format!("runner start response decode failed: {}", err),
            )
            .await;
            return;
        }
    };

    let telemetry_url = format!(
        "{}{}/{}/telemetry",
        node.trim_end_matches('/'),
        telemetry_base_path,
        started.runner_execution_id
    );
    let ack_url = format!("{}/ack", telemetry_url);
    let cancel_url = format!(
        "{}{}/{}/cancel",
        node.trim_end_matches('/'),
        telemetry_base_path,
        started.runner_execution_id
    );
    let mut after_seq = 0_u64;
    let mut poll_interval = tokio::time::interval(Duration::from_millis(
        std::env::var("PREVIA_RUNNER_LOAD_POLL_INTERVAL_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(RUNNER_LOAD_POLL_INTERVAL_MS)
            .max(50),
    ));
    poll_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let mut cancel_request = apply_runner_auth(client.post(cancel_url.clone()), runner_auth_key);
                if let Some(transaction_id) = transaction_id.as_deref() {
                    cancel_request = cancel_request.header(TRANSACTION_ID_HEADER, transaction_id);
                }
                if let Some(headers) = reservation_headers.as_ref() {
                    cancel_request = cancel_request
                        .header("x-previa-reservation-id", headers.reservation_id.as_str())
                        .header("x-previa-reservation-token", headers.reservation_token.as_str());
                }
                let _ = cancel_request.send().await;
                return;
            }
            _ = poll_interval.tick() => {}
        }

        let _poll_permit = tokio::select! {
            _ = cancel.cancelled() => {
                let mut cancel_request = apply_runner_auth(client.post(cancel_url.clone()), runner_auth_key);
                if let Some(transaction_id) = transaction_id.as_deref() {
                    cancel_request = cancel_request.header(TRANSACTION_ID_HEADER, transaction_id);
                }
                if let Some(headers) = reservation_headers.as_ref() {
                    cancel_request = cancel_request
                        .header("x-previa-reservation-id", headers.reservation_id.as_str())
                        .header("x-previa-reservation-token", headers.reservation_token.as_str());
                }
                let _ = cancel_request.send().await;
                return;
            }
            permit = poll_permits.acquire() => {
                match permit {
                    Ok(permit) => permit,
                    Err(_) => return,
                }
            }
        };

        let mut poll_request = apply_runner_auth(
            client.get(format!(
                "{}?afterSeq={}&limit={}",
                telemetry_url, after_seq, RUNNER_LOAD_POLL_LIMIT
            )),
            runner_auth_key,
        );
        if let Some(transaction_id) = transaction_id.as_deref() {
            poll_request = poll_request.header(TRANSACTION_ID_HEADER, transaction_id);
        }
        if let Some(headers) = reservation_headers.as_ref() {
            poll_request = poll_request
                .header("x-previa-reservation-id", headers.reservation_id.as_str())
                .header(
                    "x-previa-reservation-token",
                    headers.reservation_token.as_str(),
                );
        }

        let response =
            match tokio::time::timeout(Duration::from_secs(10), poll_request.send()).await {
                Ok(Ok(response)) => response,
                Ok(Err(err)) => {
                    push_load_error(
                        &load_errors,
                        format!("runner telemetry poll failed: {}", err),
                    )
                    .await;
                    continue;
                }
                Err(_) => {
                    push_load_error(&load_errors, "runner telemetry poll timeout".to_owned()).await;
                    continue;
                }
            };

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body_text = response.text().await.unwrap_or_default();
            push_load_error(
                &load_errors,
                format!("runner telemetry returned HTTP {}: {}", status, body_text),
            )
            .await;
            continue;
        }

        let telemetry = match response.json::<RunnerLoadTelemetryResponse>().await {
            Ok(telemetry) => telemetry,
            Err(err) => {
                push_load_error(
                    &load_errors,
                    format!("runner telemetry decode failed: {}", err),
                )
                .await;
                continue;
            }
        };

        let had_buckets = !telemetry.buckets.is_empty();
        let mut ack_through: Option<u64> = None;
        for bucket in telemetry.buckets {
            ack_through = Some(ack_through.unwrap_or(after_seq).max(bucket.seq));
            after_seq = after_seq.max(bucket.seq);
            if bucket.event == "execution:init" {
                continue;
            }
            record_runner_load_event(
                &node,
                bucket.event,
                bucket.payload,
                &load_chunk,
                &load_telemetry,
                &load_latency,
                &load_errors,
            )
            .await;
        }

        if let Some(ack_through) = ack_through {
            let mut ack_request = apply_runner_auth(
                client
                    .post(ack_url.clone())
                    .header("Content-Type", "application/json"),
                runner_auth_key,
            );
            if let Some(transaction_id) = transaction_id.as_deref() {
                ack_request = ack_request.header(TRANSACTION_ID_HEADER, transaction_id);
            }
            if let Some(headers) = reservation_headers.as_ref() {
                ack_request = ack_request
                    .header("x-previa-reservation-id", headers.reservation_id.as_str())
                    .header(
                        "x-previa-reservation-token",
                        headers.reservation_token.as_str(),
                    );
            }
            let _ = ack_request
                .json(&json!({ "throughSeq": ack_through }))
                .send()
                .await;
        }

        if telemetry.status != "running" && !had_buckets {
            break;
        }
    }
}

async fn emit_runner_load_request_error(
    node: &str,
    tx: &broadcast::Sender<crate::server::models::SseMessage>,
    load_errors: &Arc<Mutex<Vec<String>>>,
    load_telemetry: &Arc<Mutex<LoadTelemetryState>>,
    load_latency: &Arc<Mutex<LoadLatencyAccumulator>>,
    load_context: &Arc<LoadEventContext>,
    execution_id: &str,
    snapshot_payload: &SharedValue<Value>,
    message: String,
) {
    push_load_error(load_errors, message.clone()).await;
    refresh_load_snapshot_from_telemetry(
        execution_id,
        snapshot_payload,
        load_telemetry,
        load_latency,
        load_errors,
        load_context.as_ref(),
        "running",
    )
    .await;
    let payload = add_load_context_fields(
        json!({
            "message": message,
            "lines": [RunnerLoadLine {
                node: node.to_owned(),
                runner_event: "error".to_owned(),
                received_at: now_ms(),
                payload: json!({ "message": message }),
            }]
        }),
        load_context.as_ref(),
    );
    let _ = send_sse_best_effort(tx, "error", payload);
}

async fn record_runner_load_event(
    node: &str,
    event: String,
    data: Value,
    load_chunk: &Arc<Mutex<HashMap<String, RunnerLoadLine>>>,
    load_telemetry: &Arc<Mutex<LoadTelemetryState>>,
    load_latency: &Arc<Mutex<LoadLatencyAccumulator>>,
    load_errors: &Arc<Mutex<Vec<String>>>,
) {
    if event == "error" {
        push_load_error(load_errors, extract_error_message(&data)).await;
    }
    if event == "metrics" {
        if let Some(duration_ms) = parse_runner_duration_ms(&data) {
            let mut lock = load_latency.lock().await;
            lock.add_sample(duration_ms);
        }
        merge_runner_error_samples(load_errors, node, &data).await;
    }

    let line = RunnerLoadLine {
        node: node.to_owned(),
        runner_event: event,
        received_at: now_ms(),
        payload: data,
    };

    {
        let mut lock = load_chunk.lock().await;
        lock.insert(node.to_owned(), line.clone());
    }

    let mut telemetry_lock = load_telemetry.lock().await;
    apply_runner_telemetry_line(&mut telemetry_lock, line);
}

pub async fn flush_load_batches(
    execution_id: String,
    tx: broadcast::Sender<crate::server::models::SseMessage>,
    cancel: CancellationToken,
    stop: CancellationToken,
    load_chunk: Arc<Mutex<HashMap<String, RunnerLoadLine>>>,
    load_telemetry: Arc<Mutex<LoadTelemetryState>>,
    load_latency: Arc<Mutex<LoadLatencyAccumulator>>,
    load_errors: Arc<Mutex<Vec<String>>>,
    load_context: Arc<LoadEventContext>,
    snapshot_payload: SharedValue<Value>,
    rps_history: Arc<Mutex<BTreeMap<u64, Value>>>,
) {
    let mut interval =
        tokio::time::interval(Duration::from_millis(load_context.batch_window_ms.max(1)));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = stop.cancelled() => break,
            _ = interval.tick() => {},
        }

        let lines = drain_load_chunk(&load_chunk).await;
        if lines.is_empty() {
            continue;
        }

        let latest_snapshot = snapshot_telemetry_map(&load_telemetry).await;
        let consolidated = {
            let latency_summary = {
                let lock = load_latency.lock().await;
                summarize_load_latency(&lock)
            };
            consolidate_load_metrics(&latest_snapshot, latency_summary)
        };
        if let Some(metrics) = consolidated.as_ref() {
            if rps_history_timestamp(metrics).is_some() {
                let mut history = rps_history.lock().await;
                upsert_rps_history_samples(&mut history, metrics, &latest_snapshot);
            }
        }
        let errors = load_errors.lock().await.clone();
        let latest_lines = snapshot_telemetry_lines(&load_telemetry).await;
        snapshot_payload
            .set(build_live_load_snapshot_payload(
                &execution_id,
                "running",
                load_context.as_ref(),
                &latest_lines,
                consolidated.as_ref(),
                &errors,
            ))
            .await;
        let payload = add_load_context_fields(
            json!({ "lines": lines, "consolidated": consolidated }),
            load_context.as_ref(),
        );
        let _ = send_sse_best_effort(&tx, "metrics", payload);
    }
}

async fn merge_runner_error_samples(
    load_errors: &Arc<Mutex<Vec<String>>>,
    node: &str,
    payload: &Value,
) {
    let mut lock = load_errors.lock().await;
    merge_runner_error_samples_into(&mut lock, node, payload);
}

fn merge_runner_error_samples_into(errors: &mut Vec<String>, node: &str, payload: &Value) {
    let Some(samples) = payload.get("errorSamples").and_then(Value::as_array) else {
        return;
    };

    for sample in samples {
        let step_id = sample
            .get("stepId")
            .and_then(Value::as_str)
            .unwrap_or("unknown_step");
        let error = sample
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("pipeline failed");
        let count = sample
            .get("count")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .max(1);
        let status = sample
            .get("httpStatus")
            .and_then(Value::as_u64)
            .map(|value| format!(" HTTP {}", value))
            .unwrap_or_default();
        let key = format!("{} {}{}: {}", node, step_id, status, error);
        let message = format!("{} (x{})", key, count);

        if let Some(existing) = errors
            .iter_mut()
            .find(|existing| existing.starts_with(&key))
        {
            *existing = message;
            continue;
        }

        if errors.len() >= 20 {
            break;
        }
        errors.push(message);
    }
}

fn build_rps_history_sample(
    timestamp: u64,
    metrics: &ConsolidatedLoadMetrics,
    latest_by_node: &HashMap<String, RunnerLoadLine>,
) -> Value {
    let sample_elapsed_ms = timestamp.saturating_sub(metrics.start_time);
    let sample_bucket_ms = sample_elapsed_ms;
    let mut dispatch_bucket_total = 0usize;
    let mut has_dispatch_bucket = false;
    let mut runners = latest_by_node
        .values()
        .filter_map(|line| {
            let metrics = parse_runner_load_metrics(&line.payload)?;
            let dispatch_bucket = metrics
                .dispatch_buckets
                .iter()
                .find(|bucket| bucket.elapsed_ms == sample_bucket_ms)
                .map(|bucket| bucket.count);
            let lifecycle_bucket = metrics
                .lifecycle_buckets
                .iter()
                .find(|bucket| bucket.elapsed_ms == sample_bucket_ms);
            let mut runner = Map::new();
            runner.insert("runnerId".to_owned(), Value::String(line.node.clone()));
            insert_optional(&mut runner, "httpStarted", metrics.http_started);
            insert_optional(&mut runner, "httpCompleted", metrics.http_completed);
            insert_optional(&mut runner, "dispatchSubmitted", metrics.dispatch_submitted);
            insert_optional(&mut runner, "dispatchStarted", metrics.dispatch_started);
            insert_optional(&mut runner, "httpSendReturned", metrics.http_send_returned);
            insert_optional(
                &mut runner,
                "responseBodyCompleted",
                metrics.response_body_completed,
            );
            insert_optional(
                &mut runner,
                "dependencyLimitedStarts",
                metrics.dependency_limited_starts,
            );
            insert_optional(
                &mut runner,
                "dispatcherLaggedStarts",
                metrics.dispatcher_lagged_starts,
            );
            insert_optional(
                &mut runner,
                "runtimeLaggedStarts",
                metrics.runtime_lagged_starts,
            );
            insert_optional(
                &mut runner,
                "senderLaggedStarts",
                metrics.sender_lagged_starts,
            );
            insert_optional(&mut runner, "senderQueueDepth", metrics.sender_queue_depth);
            insert_optional(
                &mut runner,
                "senderStartLagP95Ms",
                metrics.sender_start_lag_p95_ms,
            );
            insert_optional(
                &mut runner,
                "httpSendDurationP95Ms",
                metrics.http_send_duration_p95_ms,
            );
            insert_optional(
                &mut runner,
                "responseObservationDurationP95Ms",
                metrics.response_observation_duration_p95_ms,
            );
            insert_optional(&mut runner, "schedulerLagMs", metrics.scheduler_lag_ms);
            insert_optional(
                &mut runner,
                "schedulerLaggedStarts",
                metrics.scheduler_lagged_starts,
            );
            insert_optional(&mut runner, "slotEnqueued", metrics.slot_enqueued);
            insert_optional(&mut runner, "requestPrepared", metrics.request_prepared);
            insert_optional(&mut runner, "requestEnqueued", metrics.request_enqueued);
            insert_optional(&mut runner, "sendTaskSpawned", metrics.send_task_spawned);
            insert_optional(&mut runner, "sendStarted", metrics.send_started);
            insert_optional(&mut runner, "totalStarted", metrics.total_started);
            runner.insert("totalSent".to_owned(), json!(metrics.total_sent));
            runner.insert("rps".to_owned(), json!(metrics.rps));
            insert_optional(&mut runner, "scheduledStarts", metrics.scheduled_starts);
            insert_optional(&mut runner, "missedStarts", metrics.missed_starts);
            insert_optional(&mut runner, "readyRequests", metrics.ready_requests);
            insert_optional(&mut runner, "activePipelines", metrics.active_pipelines);
            insert_optional(
                &mut runner,
                "outstandingRequests",
                metrics.outstanding_requests,
            );
            insert_optional(&mut runner, "curveAdherence", metrics.curve_adherence);
            if let Some(count) = dispatch_bucket {
                dispatch_bucket_total = dispatch_bucket_total.saturating_add(count);
                has_dispatch_bucket = true;
                runner.insert("dispatchBucket".to_owned(), json!(count));
            }
            if let Some(bucket) = lifecycle_bucket {
                runner.insert(
                    "lifecycleBucket".to_owned(),
                    json!({
                        "elapsedMs": bucket.elapsed_ms,
                        "planned": bucket.planned,
                        "slotEnqueued": bucket.slot_enqueued,
                        "requestPrepared": bucket.request_prepared,
                        "requestEnqueued": bucket.request_enqueued,
                        "sendTaskSpawned": bucket.send_task_spawned,
                        "sendStarted": bucket.send_started,
                        "httpStarted": bucket.http_started,
                        "httpSendReturned": bucket.http_send_returned,
                        "responseBodyCompleted": bucket.response_body_completed,
                        "dispatcherLagged": bucket.dispatcher_lagged,
                        "runtimeLagged": bucket.runtime_lagged,
                        "senderLagged": bucket.sender_lagged,
                    }),
                );
            }
            Some(Value::Object(runner))
        })
        .collect::<Vec<_>>();
    runners.sort_by(|a, b| {
        a.get("runnerId")
            .and_then(Value::as_str)
            .cmp(&b.get("runnerId").and_then(Value::as_str))
    });

    let mut sample = Map::new();
    sample.insert("timestamp".to_owned(), json!(timestamp));
    sample.insert(
        "elapsedMs".to_owned(),
        json!(timestamp.saturating_sub(metrics.start_time)),
    );
    sample.insert("rps".to_owned(), json!(metrics.rps));
    insert_optional(&mut sample, "totalStarted", metrics.total_started);
    sample.insert("totalSent".to_owned(), json!(metrics.total_sent));
    insert_optional(&mut sample, "httpStarted", metrics.http_started);
    insert_optional(&mut sample, "httpCompleted", metrics.http_completed);
    insert_optional(&mut sample, "dispatchSubmitted", metrics.dispatch_submitted);
    insert_optional(&mut sample, "dispatchStarted", metrics.dispatch_started);
    insert_optional(&mut sample, "httpSendReturned", metrics.http_send_returned);
    insert_optional(
        &mut sample,
        "responseBodyCompleted",
        metrics.response_body_completed,
    );
    insert_optional(
        &mut sample,
        "dependencyLimitedStarts",
        metrics.dependency_limited_starts,
    );
    insert_optional(
        &mut sample,
        "dispatcherLaggedStarts",
        metrics.dispatcher_lagged_starts,
    );
    insert_optional(
        &mut sample,
        "runtimeLaggedStarts",
        metrics.runtime_lagged_starts,
    );
    insert_optional(
        &mut sample,
        "senderLaggedStarts",
        metrics.sender_lagged_starts,
    );
    insert_optional(&mut sample, "senderQueueDepth", metrics.sender_queue_depth);
    insert_optional(
        &mut sample,
        "senderStartLagP95Ms",
        metrics.sender_start_lag_p95_ms,
    );
    insert_optional(
        &mut sample,
        "httpSendDurationP95Ms",
        metrics.http_send_duration_p95_ms,
    );
    insert_optional(
        &mut sample,
        "responseObservationDurationP95Ms",
        metrics.response_observation_duration_p95_ms,
    );
    insert_optional(&mut sample, "schedulerLagMs", metrics.scheduler_lag_ms);
    insert_optional(
        &mut sample,
        "schedulerLaggedStarts",
        metrics.scheduler_lagged_starts,
    );
    insert_optional(&mut sample, "slotEnqueued", metrics.slot_enqueued);
    insert_optional(&mut sample, "requestPrepared", metrics.request_prepared);
    insert_optional(&mut sample, "requestEnqueued", metrics.request_enqueued);
    insert_optional(&mut sample, "sendTaskSpawned", metrics.send_task_spawned);
    insert_optional(&mut sample, "sendStarted", metrics.send_started);
    if has_dispatch_bucket {
        sample.insert("dispatchBucket".to_owned(), json!(dispatch_bucket_total));
    }
    if let Some(bucket) = metrics
        .lifecycle_buckets
        .iter()
        .find(|bucket| bucket.elapsed_ms == sample_bucket_ms)
    {
        sample.insert("lifecycleBucket".to_owned(), json!(bucket));
    }
    insert_optional(&mut sample, "targetIntensity", metrics.target_intensity);
    insert_optional(&mut sample, "targetRpsLimit", metrics.target_rps_limit);
    insert_optional(&mut sample, "scheduledStarts", metrics.scheduled_starts);
    insert_optional(&mut sample, "missedStarts", metrics.missed_starts);
    insert_optional(&mut sample, "readyRequests", metrics.ready_requests);
    insert_optional(&mut sample, "activePipelines", metrics.active_pipelines);
    insert_optional(
        &mut sample,
        "outstandingRequests",
        metrics.outstanding_requests,
    );
    insert_optional(&mut sample, "curveAdherence", metrics.curve_adherence);
    sample.insert("runners".to_owned(), Value::Array(runners));
    Value::Object(sample)
}

pub fn upsert_rps_history_samples(
    history: &mut BTreeMap<u64, Value>,
    metrics: &ConsolidatedLoadMetrics,
    latest_by_node: &HashMap<String, RunnerLoadLine>,
) {
    let Some(current_sample_at) = rps_history_timestamp(metrics) else {
        return;
    };
    let first_sample_at = current_sample_at.saturating_sub(RPS_HISTORY_CORRECTION_WINDOW_MS);
    let mut sample_at = first_sample_at;
    while sample_at <= current_sample_at {
        history.insert(
            sample_at,
            build_rps_history_sample(sample_at, metrics, latest_by_node),
        );
        sample_at = sample_at.saturating_add(RPS_HISTORY_BUCKET_MS);
        if sample_at == 0 {
            break;
        }
    }
}

pub fn rebuild_final_rps_history(
    metrics: &ConsolidatedLoadMetrics,
    latest_by_node: &HashMap<String, RunnerLoadLine>,
) -> Vec<Value> {
    let mut bucket_ms = BTreeMap::<u64, ()>::new();
    for bucket in &metrics.lifecycle_buckets {
        bucket_ms.insert(bucket.elapsed_ms, ());
    }
    for line in latest_by_node.values() {
        let Some(metrics) = parse_runner_load_metrics(&line.payload) else {
            continue;
        };
        for bucket in metrics.dispatch_buckets {
            bucket_ms.insert(bucket.elapsed_ms, ());
        }
    }

    bucket_ms
        .keys()
        .map(|elapsed_ms| {
            build_rps_history_sample(
                metrics.start_time.saturating_add(*elapsed_ms),
                metrics,
                latest_by_node,
            )
        })
        .collect()
}

fn rps_history_timestamp(metrics: &ConsolidatedLoadMetrics) -> Option<u64> {
    (metrics.elapsed_ms >= RPS_HISTORY_BUCKET_MS).then(|| {
        metrics
            .start_time
            .saturating_add(rps_history_elapsed_bucket_ms(metrics.elapsed_ms))
    })
}

fn rps_history_elapsed_bucket_ms(elapsed_ms: u64) -> u64 {
    elapsed_ms
        .saturating_sub(RPS_HISTORY_BUCKET_MS)
        .checked_div(RPS_HISTORY_BUCKET_MS)
        .unwrap_or(0)
        .saturating_mul(RPS_HISTORY_BUCKET_MS)
}

fn insert_optional<T: Serialize>(map: &mut Map<String, Value>, key: &str, value: Option<T>) {
    if let Some(value) = value {
        map.insert(key.to_owned(), json!(value));
    }
}

fn max_optional_u64(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

fn max_optional_f64(left: Option<f64>, right: Option<f64>) -> Option<f64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

async fn refresh_load_snapshot_from_telemetry(
    execution_id: &str,
    snapshot_payload: &SharedValue<Value>,
    load_telemetry: &Arc<Mutex<LoadTelemetryState>>,
    load_latency: &Arc<Mutex<LoadLatencyAccumulator>>,
    load_errors: &Arc<Mutex<Vec<String>>>,
    load_context: &LoadEventContext,
    status: &str,
) {
    let lines = snapshot_telemetry_lines(load_telemetry).await;
    let consolidated = snapshot_telemetry_consolidated_metrics(load_telemetry, load_latency).await;
    let errors = load_errors.lock().await.clone();
    snapshot_payload
        .set(build_live_load_snapshot_payload(
            execution_id,
            status,
            load_context,
            &lines,
            consolidated.as_ref(),
            &errors,
        ))
        .await;
}

pub fn apply_runner_telemetry_line(state: &mut LoadTelemetryState, mut line: RunnerLoadLine) {
    let Some(metrics) = parse_runner_load_metrics(&line.payload) else {
        let node = line.node.clone();
        let runner = state.runners.entry(node).or_default();
        if runner.line.is_none() {
            runner.line = Some(line);
        }
        return;
    };

    let runner = state.runners.entry(line.node.clone()).or_default();
    let replaces_bucket_state = metrics.snapshot_mode != Some(RunnerLoadSnapshotMode::Live);
    if replaces_bucket_state {
        runner.dispatch_buckets.clear();
        runner.lifecycle_buckets.clear();
        runner.status_code_buckets.clear();
    }

    for bucket in metrics.dispatch_buckets {
        runner.dispatch_buckets.insert(bucket.elapsed_ms, bucket);
    }
    for bucket in metrics.lifecycle_buckets {
        runner.lifecycle_buckets.insert(bucket.elapsed_ms, bucket);
    }
    for bucket in metrics.status_code_buckets {
        runner
            .status_code_buckets
            .insert((bucket.elapsed_ms, bucket.code.clone()), bucket);
    }

    line.payload = payload_with_merged_buckets(
        &line.payload,
        &runner.dispatch_buckets,
        &runner.lifecycle_buckets,
        &runner.status_code_buckets,
    );
    runner.line = Some(line);
}

fn payload_with_merged_buckets(
    payload: &Value,
    dispatch_buckets: &BTreeMap<u64, RunnerLoadDispatchBucket>,
    lifecycle_buckets: &BTreeMap<u64, RunnerLoadLifecycleBucket>,
    status_code_buckets: &BTreeMap<(u64, String), RunnerLoadStatusCodeBucket>,
) -> Value {
    let mut obj = payload.as_object().cloned().unwrap_or_default();
    obj.insert(
        "dispatchBuckets".to_owned(),
        Value::Array(
            dispatch_buckets
                .values()
                .map(|bucket| {
                    json!({
                        "elapsedMs": bucket.elapsed_ms,
                        "count": bucket.count,
                    })
                })
                .collect(),
        ),
    );
    obj.insert(
        "lifecycleBuckets".to_owned(),
        Value::Array(
            lifecycle_buckets
                .values()
                .map(|bucket| {
                    json!({
                        "elapsedMs": bucket.elapsed_ms,
                        "planned": bucket.planned,
                        "slotEnqueued": bucket.slot_enqueued,
                        "requestPrepared": bucket.request_prepared,
                        "requestEnqueued": bucket.request_enqueued,
                        "sendTaskSpawned": bucket.send_task_spawned,
                        "sendStarted": bucket.send_started,
                        "httpStarted": bucket.http_started,
                        "httpSendReturned": bucket.http_send_returned,
                        "responseBodyCompleted": bucket.response_body_completed,
                        "dispatcherLagged": bucket.dispatcher_lagged,
                        "runtimeLagged": bucket.runtime_lagged,
                        "senderLagged": bucket.sender_lagged,
                    })
                })
                .collect(),
        ),
    );
    obj.insert(
        "statusCodeBuckets".to_owned(),
        Value::Array(
            status_code_buckets
                .values()
                .map(|bucket| {
                    json!({
                        "elapsedMs": bucket.elapsed_ms,
                        "code": bucket.code,
                        "count": bucket.count,
                    })
                })
                .collect(),
        ),
    );
    Value::Object(obj)
}

pub async fn drain_load_chunk(
    load_chunk: &Arc<Mutex<HashMap<String, RunnerLoadLine>>>,
) -> Vec<RunnerLoadLine> {
    let mut lock = load_chunk.lock().await;
    let mut lines: Vec<RunnerLoadLine> = lock.drain().map(|(_, line)| line).collect();
    lines.sort_by(|a, b| a.node.cmp(&b.node));
    lines
}

pub async fn snapshot_telemetry_lines(
    load_telemetry: &Arc<Mutex<LoadTelemetryState>>,
) -> Vec<RunnerLoadLine> {
    let lock = load_telemetry.lock().await;
    let mut lines: Vec<RunnerLoadLine> = lock
        .runners
        .values()
        .filter_map(|runner| runner.line.clone())
        .collect();
    lines.sort_by(|a, b| a.node.cmp(&b.node));
    lines
}

pub async fn snapshot_telemetry_map(
    load_telemetry: &Arc<Mutex<LoadTelemetryState>>,
) -> HashMap<String, RunnerLoadLine> {
    snapshot_telemetry_lines(load_telemetry)
        .await
        .into_iter()
        .map(|line| (line.node.clone(), line))
        .collect()
}

pub async fn snapshot_telemetry_consolidated_metrics(
    load_telemetry: &Arc<Mutex<LoadTelemetryState>>,
    load_latency: &Arc<Mutex<LoadLatencyAccumulator>>,
) -> Option<ConsolidatedLoadMetrics> {
    let latest_snapshot = snapshot_telemetry_map(load_telemetry).await;
    let latency_summary = {
        let lock = load_latency.lock().await;
        summarize_load_latency(&lock)
    };
    consolidate_load_metrics(&latest_snapshot, latency_summary)
}

pub fn consolidate_load_metrics(
    latest_by_node: &HashMap<String, RunnerLoadLine>,
    latency: LoadLatencySummary,
) -> Option<ConsolidatedLoadMetrics> {
    let latency = summarize_runner_latency(latest_by_node).unwrap_or(latency);
    let mut total_sent = 0usize;
    let mut total_started = 0usize;
    let mut total_started_nodes = 0usize;
    let mut total_success = 0usize;
    let mut total_error = 0usize;
    let mut http_started = 0usize;
    let mut http_started_nodes = 0usize;
    let mut http_completed = 0usize;
    let mut http_completed_nodes = 0usize;
    let mut dispatch_submitted = 0usize;
    let mut dispatch_submitted_nodes = 0usize;
    let mut dispatch_started = 0usize;
    let mut dispatch_started_nodes = 0usize;
    let mut http_send_returned = 0usize;
    let mut http_send_returned_nodes = 0usize;
    let mut response_body_completed = 0usize;
    let mut response_body_completed_nodes = 0usize;
    let mut dependency_limited_starts = 0usize;
    let mut dependency_limited_starts_nodes = 0usize;
    let mut dispatcher_lagged_starts = 0usize;
    let mut dispatcher_lagged_starts_nodes = 0usize;
    let mut runtime_lagged_starts = 0usize;
    let mut runtime_lagged_starts_nodes = 0usize;
    let mut sender_lagged_starts = 0usize;
    let mut sender_lagged_starts_nodes = 0usize;
    let mut sender_queue_depth = 0usize;
    let mut sender_queue_depth_nodes = 0usize;
    let mut sender_start_lag_avg_ms: Option<f64> = None;
    let mut sender_start_lag_p95_ms: Option<u64> = None;
    let mut sender_start_lag_p99_ms: Option<u64> = None;
    let mut sender_start_lag_max_ms: Option<u64> = None;
    let mut http_send_duration_avg_ms: Option<f64> = None;
    let mut http_send_duration_p95_ms: Option<u64> = None;
    let mut http_send_duration_p99_ms: Option<u64> = None;
    let mut response_observation_duration_avg_ms: Option<f64> = None;
    let mut response_observation_duration_p95_ms: Option<u64> = None;
    let mut response_observation_duration_p99_ms: Option<u64> = None;
    let mut scheduler_lag_ms = 0u64;
    let mut scheduler_lag_ms_nodes = 0usize;
    let mut scheduler_lagged_starts = 0usize;
    let mut scheduler_lagged_starts_nodes = 0usize;
    let mut slot_enqueued = 0usize;
    let mut slot_enqueued_nodes = 0usize;
    let mut request_prepared = 0usize;
    let mut request_prepared_nodes = 0usize;
    let mut request_enqueued = 0usize;
    let mut request_enqueued_nodes = 0usize;
    let mut send_task_spawned = 0usize;
    let mut send_task_spawned_nodes = 0usize;
    let mut send_started = 0usize;
    let mut send_started_nodes = 0usize;
    let mut rps = 0.0f64;
    let mut target_intensity = 0.0f64;
    let mut target_intensity_nodes = 0usize;
    let mut target_rps_limit = 0.0f64;
    let mut in_flight = 0usize;
    let mut runner_max_rps = 0.0f64;
    let mut tick_ms = 0u64;
    let mut scheduled_starts = 0usize;
    let mut scheduled_starts_nodes = 0usize;
    let mut missed_starts = 0usize;
    let mut missed_starts_nodes = 0usize;
    let mut ready_requests = 0usize;
    let mut ready_requests_nodes = 0usize;
    let mut active_pipelines = 0usize;
    let mut active_pipelines_nodes = 0usize;
    let mut outstanding_requests = 0usize;
    let mut outstanding_requests_nodes = 0usize;
    let mut start_time = u64::MAX;
    let mut elapsed_ms = 0u64;
    let mut nodes_reporting = 0usize;
    let mut lifecycle_by_elapsed = BTreeMap::<u64, ConsolidatedLoadLifecycleBucket>::new();
    let mut status_codes_by_elapsed =
        BTreeMap::<(u64, String), ConsolidatedLoadStatusCodeBucket>::new();

    for line in latest_by_node.values() {
        let Some(metrics) = parse_runner_load_metrics(&line.payload) else {
            continue;
        };

        for bucket in &metrics.lifecycle_buckets {
            let entry = lifecycle_by_elapsed
                .entry(bucket.elapsed_ms)
                .or_insert_with(|| ConsolidatedLoadLifecycleBucket {
                    elapsed_ms: bucket.elapsed_ms,
                    planned: 0,
                    slot_enqueued: 0,
                    request_prepared: 0,
                    request_enqueued: 0,
                    send_task_spawned: 0,
                    send_started: 0,
                    http_started: 0,
                    http_send_returned: 0,
                    response_body_completed: 0,
                    dispatcher_lagged: 0,
                    runtime_lagged: 0,
                    sender_lagged: 0,
                    sender_start_lag_ms_max: 0,
                    http_send_duration_ms_max: 0,
                    response_observation_duration_ms_max: 0,
                });
            entry.planned = entry.planned.saturating_add(bucket.planned);
            entry.slot_enqueued = entry.slot_enqueued.saturating_add(bucket.slot_enqueued);
            entry.request_prepared = entry
                .request_prepared
                .saturating_add(bucket.request_prepared);
            entry.request_enqueued = entry
                .request_enqueued
                .saturating_add(bucket.request_enqueued);
            entry.send_task_spawned = entry
                .send_task_spawned
                .saturating_add(bucket.send_task_spawned);
            entry.send_started = entry.send_started.saturating_add(bucket.send_started);
            entry.http_started = entry.http_started.saturating_add(bucket.http_started);
            entry.http_send_returned = entry
                .http_send_returned
                .saturating_add(bucket.http_send_returned);
            entry.response_body_completed = entry
                .response_body_completed
                .saturating_add(bucket.response_body_completed);
            entry.dispatcher_lagged = entry
                .dispatcher_lagged
                .saturating_add(bucket.dispatcher_lagged);
            entry.runtime_lagged = entry.runtime_lagged.saturating_add(bucket.runtime_lagged);
            entry.sender_lagged = entry.sender_lagged.saturating_add(bucket.sender_lagged);
            entry.sender_start_lag_ms_max = entry
                .sender_start_lag_ms_max
                .max(bucket.sender_start_lag_ms_max);
            entry.http_send_duration_ms_max = entry
                .http_send_duration_ms_max
                .max(bucket.http_send_duration_ms_max);
            entry.response_observation_duration_ms_max = entry
                .response_observation_duration_ms_max
                .max(bucket.response_observation_duration_ms_max);
        }
        for bucket in &metrics.status_code_buckets {
            let entry = status_codes_by_elapsed
                .entry((bucket.elapsed_ms, bucket.code.clone()))
                .or_insert_with(|| ConsolidatedLoadStatusCodeBucket {
                    elapsed_ms: bucket.elapsed_ms,
                    code: bucket.code.clone(),
                    count: 0,
                });
            entry.count = entry.count.saturating_add(bucket.count);
        }

        if let Some(value) = metrics.total_started {
            total_started = total_started.saturating_add(value);
            total_started_nodes += 1;
        }
        total_sent = total_sent.saturating_add(metrics.total_sent);
        total_success = total_success.saturating_add(metrics.total_success);
        total_error = total_error.saturating_add(metrics.total_error);
        if let Some(value) = metrics.http_started {
            http_started = http_started.saturating_add(value);
            http_started_nodes += 1;
        }
        if let Some(value) = metrics.http_completed {
            http_completed = http_completed.saturating_add(value);
            http_completed_nodes += 1;
        }
        if let Some(value) = metrics.dispatch_submitted {
            dispatch_submitted = dispatch_submitted.saturating_add(value);
            dispatch_submitted_nodes += 1;
        }
        if let Some(value) = metrics.dispatch_started {
            dispatch_started = dispatch_started.saturating_add(value);
            dispatch_started_nodes += 1;
        }
        if let Some(value) = metrics.http_send_returned {
            http_send_returned = http_send_returned.saturating_add(value);
            http_send_returned_nodes += 1;
        }
        if let Some(value) = metrics.response_body_completed {
            response_body_completed = response_body_completed.saturating_add(value);
            response_body_completed_nodes += 1;
        }
        if let Some(value) = metrics.dependency_limited_starts {
            dependency_limited_starts = dependency_limited_starts.saturating_add(value);
            dependency_limited_starts_nodes += 1;
        }
        if let Some(value) = metrics.dispatcher_lagged_starts {
            dispatcher_lagged_starts = dispatcher_lagged_starts.saturating_add(value);
            dispatcher_lagged_starts_nodes += 1;
        }
        if let Some(value) = metrics.runtime_lagged_starts {
            runtime_lagged_starts = runtime_lagged_starts.saturating_add(value);
            runtime_lagged_starts_nodes += 1;
        }
        if let Some(value) = metrics.sender_lagged_starts {
            sender_lagged_starts = sender_lagged_starts.saturating_add(value);
            sender_lagged_starts_nodes += 1;
        }
        if let Some(value) = metrics.sender_queue_depth {
            sender_queue_depth = sender_queue_depth.saturating_add(value);
            sender_queue_depth_nodes += 1;
        }
        sender_start_lag_avg_ms =
            max_optional_f64(sender_start_lag_avg_ms, metrics.sender_start_lag_avg_ms);
        sender_start_lag_p95_ms =
            max_optional_u64(sender_start_lag_p95_ms, metrics.sender_start_lag_p95_ms);
        sender_start_lag_p99_ms =
            max_optional_u64(sender_start_lag_p99_ms, metrics.sender_start_lag_p99_ms);
        sender_start_lag_max_ms =
            max_optional_u64(sender_start_lag_max_ms, metrics.sender_start_lag_max_ms);
        http_send_duration_avg_ms =
            max_optional_f64(http_send_duration_avg_ms, metrics.http_send_duration_avg_ms);
        http_send_duration_p95_ms =
            max_optional_u64(http_send_duration_p95_ms, metrics.http_send_duration_p95_ms);
        http_send_duration_p99_ms =
            max_optional_u64(http_send_duration_p99_ms, metrics.http_send_duration_p99_ms);
        response_observation_duration_avg_ms = max_optional_f64(
            response_observation_duration_avg_ms,
            metrics.response_observation_duration_avg_ms,
        );
        response_observation_duration_p95_ms = max_optional_u64(
            response_observation_duration_p95_ms,
            metrics.response_observation_duration_p95_ms,
        );
        response_observation_duration_p99_ms = max_optional_u64(
            response_observation_duration_p99_ms,
            metrics.response_observation_duration_p99_ms,
        );
        if let Some(value) = metrics.scheduler_lag_ms {
            scheduler_lag_ms = scheduler_lag_ms.saturating_add(value);
            scheduler_lag_ms_nodes += 1;
        }
        if let Some(value) = metrics.scheduler_lagged_starts {
            scheduler_lagged_starts = scheduler_lagged_starts.saturating_add(value);
            scheduler_lagged_starts_nodes += 1;
        }
        if let Some(value) = metrics.slot_enqueued {
            slot_enqueued = slot_enqueued.saturating_add(value);
            slot_enqueued_nodes += 1;
        }
        if let Some(value) = metrics.request_prepared {
            request_prepared = request_prepared.saturating_add(value);
            request_prepared_nodes += 1;
        }
        if let Some(value) = metrics.request_enqueued {
            request_enqueued = request_enqueued.saturating_add(value);
            request_enqueued_nodes += 1;
        }
        if let Some(value) = metrics.send_task_spawned {
            send_task_spawned = send_task_spawned.saturating_add(value);
            send_task_spawned_nodes += 1;
        }
        if let Some(value) = metrics.send_started {
            send_started = send_started.saturating_add(value);
            send_started_nodes += 1;
        }
        rps += metrics.rps;
        if let Some(value) = metrics.target_intensity {
            target_intensity += value;
            target_intensity_nodes += 1;
        }
        if let Some(value) = metrics.target_rps_limit {
            target_rps_limit += value;
        }
        if let Some(value) = metrics.in_flight {
            in_flight = in_flight.saturating_add(value);
        }
        if let Some(value) = metrics.runner_max_rps {
            runner_max_rps += value;
        }
        if let Some(value) = metrics.tick_ms {
            tick_ms = tick_ms.max(value);
        }
        if let Some(value) = metrics.scheduled_starts {
            scheduled_starts = scheduled_starts.saturating_add(value);
            scheduled_starts_nodes += 1;
        }
        if let Some(value) = metrics.missed_starts {
            missed_starts = missed_starts.saturating_add(value);
            missed_starts_nodes += 1;
        }
        if let Some(value) = metrics.ready_requests {
            ready_requests = ready_requests.saturating_add(value);
            ready_requests_nodes += 1;
        }
        if let Some(value) = metrics.active_pipelines {
            active_pipelines = active_pipelines.saturating_add(value);
            active_pipelines_nodes += 1;
        }
        if let Some(value) = metrics.outstanding_requests {
            outstanding_requests = outstanding_requests.saturating_add(value);
            outstanding_requests_nodes += 1;
        }
        start_time = start_time.min(metrics.start_time);
        elapsed_ms = elapsed_ms.max(metrics.elapsed_ms);
        nodes_reporting += 1;
    }

    if nodes_reporting == 0 {
        return None;
    }
    let temporal_curve_adherence = lifecycle_curve_adherence(&lifecycle_by_elapsed);

    Some(ConsolidatedLoadMetrics {
        total_started: (total_started_nodes > 0).then_some(total_started),
        total_sent,
        total_success,
        total_error,
        http_started: (http_started_nodes > 0).then_some(http_started),
        http_completed: (http_completed_nodes > 0).then_some(http_completed),
        dispatch_submitted: (dispatch_submitted_nodes > 0).then_some(dispatch_submitted),
        dispatch_started: (dispatch_started_nodes > 0).then_some(dispatch_started),
        http_send_returned: (http_send_returned_nodes > 0).then_some(http_send_returned),
        response_body_completed: (response_body_completed_nodes > 0)
            .then_some(response_body_completed),
        dependency_limited_starts: (dependency_limited_starts_nodes > 0)
            .then_some(dependency_limited_starts),
        dispatcher_lagged_starts: (dispatcher_lagged_starts_nodes > 0)
            .then_some(dispatcher_lagged_starts),
        runtime_lagged_starts: (runtime_lagged_starts_nodes > 0).then_some(runtime_lagged_starts),
        sender_lagged_starts: (sender_lagged_starts_nodes > 0).then_some(sender_lagged_starts),
        sender_queue_depth: (sender_queue_depth_nodes > 0).then_some(sender_queue_depth),
        sender_start_lag_avg_ms,
        sender_start_lag_p95_ms,
        sender_start_lag_p99_ms,
        sender_start_lag_max_ms,
        http_send_duration_avg_ms,
        http_send_duration_p95_ms,
        http_send_duration_p99_ms,
        response_observation_duration_avg_ms,
        response_observation_duration_p95_ms,
        response_observation_duration_p99_ms,
        scheduler_lag_ms: (scheduler_lag_ms_nodes > 0).then_some(scheduler_lag_ms),
        scheduler_lagged_starts: (scheduler_lagged_starts_nodes > 0)
            .then_some(scheduler_lagged_starts),
        slot_enqueued: (slot_enqueued_nodes > 0).then_some(slot_enqueued),
        request_prepared: (request_prepared_nodes > 0).then_some(request_prepared),
        request_enqueued: (request_enqueued_nodes > 0).then_some(request_enqueued),
        send_task_spawned: (send_task_spawned_nodes > 0).then_some(send_task_spawned),
        send_started: (send_started_nodes > 0).then_some(send_started),
        rps,
        target_intensity: (target_intensity_nodes > 0)
            .then(|| target_intensity / target_intensity_nodes as f64),
        target_rps_limit: (target_rps_limit > 0.0).then_some(target_rps_limit),
        in_flight: (in_flight > 0).then_some(in_flight),
        runner_max_rps: (runner_max_rps > 0.0).then_some(runner_max_rps),
        tick_ms: (tick_ms > 0).then_some(tick_ms),
        scheduled_starts: (scheduled_starts_nodes > 0).then_some(scheduled_starts),
        missed_starts: (missed_starts_nodes > 0).then_some(missed_starts),
        ready_requests: (ready_requests_nodes > 0).then_some(ready_requests),
        active_pipelines: (active_pipelines_nodes > 0).then_some(active_pipelines),
        outstanding_requests: (outstanding_requests_nodes > 0).then_some(outstanding_requests),
        curve_adherence: temporal_curve_adherence.or_else(|| {
            (scheduled_starts_nodes > 0).then(|| {
                if scheduled_starts == 0 {
                    100.0
                } else {
                    let value = ((scheduled_starts.saturating_sub(missed_starts)) as f64
                        / scheduled_starts as f64)
                        * 100.0;
                    (value * 100.0).round() / 100.0
                }
            })
        }),
        avg_latency: latency.avg_latency,
        p95: latency.p95,
        p99: latency.p99,
        start_time,
        elapsed_ms,
        nodes_reporting,
        lifecycle_buckets: lifecycle_by_elapsed.into_values().collect(),
        status_code_buckets: status_codes_by_elapsed.into_values().collect(),
    })
}

fn lifecycle_curve_adherence(
    lifecycle_by_elapsed: &BTreeMap<u64, ConsolidatedLoadLifecycleBucket>,
) -> Option<f64> {
    let planned_total: usize = lifecycle_by_elapsed
        .values()
        .map(|bucket| bucket.planned)
        .sum();
    if planned_total == 0 {
        return None;
    }

    let absolute_error: usize = lifecycle_by_elapsed
        .values()
        .map(|bucket| bucket.planned.abs_diff(bucket.send_started))
        .sum();
    let raw = (1.0 - (absolute_error as f64 / planned_total as f64)).max(0.0) * 100.0;
    Some((raw * 100.0).round() / 100.0)
}

fn summarize_runner_latency(
    latest_by_node: &HashMap<String, RunnerLoadLine>,
) -> Option<LoadLatencySummary> {
    let mut accumulator = LoadLatencyAccumulator::default();

    for line in latest_by_node.values() {
        let Some(metrics) = parse_runner_load_metrics(&line.payload) else {
            continue;
        };

        let reported_sample_count = metrics.latency_sample_count.unwrap_or(0);
        let reported_total_duration_ms = metrics.latency_total_duration_ms.unwrap_or(0);
        let mut bucket_sample_count = 0usize;
        for bucket in metrics.latency_buckets {
            bucket_sample_count = bucket_sample_count.saturating_add(bucket.count);
            accumulator.sample_count = accumulator.sample_count.saturating_add(bucket.count);
            accumulator.total_duration_ms = accumulator
                .total_duration_ms
                .saturating_add((bucket.duration_ms as u128).saturating_mul(bucket.count as u128));
            *accumulator.histogram.entry(bucket.duration_ms).or_insert(0) += bucket.count;
        }

        if bucket_sample_count == 0 && reported_sample_count > 0 {
            let duration_ms =
                round_average_latency(reported_total_duration_ms as u128, reported_sample_count);
            accumulator.sample_count = accumulator
                .sample_count
                .saturating_add(reported_sample_count);
            accumulator.total_duration_ms = accumulator
                .total_duration_ms
                .saturating_add(reported_total_duration_ms as u128);
            *accumulator.histogram.entry(duration_ms).or_insert(0) += reported_sample_count;
        }
    }

    (accumulator.sample_count > 0).then(|| summarize_load_latency(&accumulator))
}

pub fn summarize_load_latency(accumulator: &LoadLatencyAccumulator) -> LoadLatencySummary {
    if accumulator.sample_count == 0 {
        return LoadLatencySummary::default();
    }

    let avg_latency =
        round_average_latency(accumulator.total_duration_ms, accumulator.sample_count);
    let p95 = percentile_from_histogram(&accumulator.histogram, accumulator.sample_count, 95, 100);
    let p99 = percentile_from_histogram(&accumulator.histogram, accumulator.sample_count, 99, 100);

    LoadLatencySummary {
        avg_latency,
        p95,
        p99,
    }
}

pub fn round_average_latency(total_duration_ms: u128, sample_count: usize) -> u64 {
    if sample_count == 0 {
        return 0;
    }

    let count = sample_count as u128;
    let rounded = total_duration_ms
        .saturating_add(count / 2)
        .saturating_div(count);
    u64::try_from(rounded).unwrap_or(u64::MAX)
}

pub fn percentile_from_histogram(
    histogram: &BTreeMap<u64, usize>,
    sample_count: usize,
    numerator: u64,
    denominator: u64,
) -> u64 {
    if sample_count == 0 || histogram.is_empty() || denominator == 0 {
        return 0;
    }

    let rank =
        (((sample_count as u128) * (numerator as u128)) / (denominator as u128)).saturating_add(1);
    let rank = usize::try_from(rank).unwrap_or(usize::MAX);

    let mut cumulative = 0usize;
    let mut last_bucket = 0u64;
    for (bucket, count) in histogram {
        last_bucket = *bucket;
        cumulative = cumulative.saturating_add(*count);
        if cumulative >= rank {
            return *bucket;
        }
    }

    last_bucket
}

pub fn add_load_context_fields(data: Value, context: &LoadEventContext) -> Value {
    let mut object = match data {
        Value::Object(obj) => obj,
        other => {
            let mut obj = Map::new();
            obj.insert("payload".to_owned(), other);
            obj
        }
    };

    object.insert(
        "requestedNodes".to_owned(),
        json!(context.plan.requested_nodes),
    );
    object.insert("nodesFound".to_owned(), json!(context.plan.nodes_found));
    object.insert("nodesUsed".to_owned(), json!(context.plan.nodes_used));

    object.insert(
        "registeredNodesTotal".to_owned(),
        json!(context.registered_nodes.len()),
    );
    object.insert(
        "activeNodesTotal".to_owned(),
        json!(context.active_nodes.len()),
    );
    object.insert("usedNodesTotal".to_owned(), json!(context.used_nodes.len()));
    object.insert(
        "registeredNodes".to_owned(),
        json!(&context.registered_nodes),
    );
    object.insert("activeNodes".to_owned(), json!(&context.active_nodes));
    object.insert("usedNodes".to_owned(), json!(&context.used_nodes));
    object.insert(
        "runnerLoadPlan".to_owned(),
        json!(&context.runner_load_plan),
    );
    object.insert("batchWindowMs".to_owned(), json!(context.batch_window_ms));

    if let Some(warning) = context.warning.as_ref().or(context.plan.warning.as_ref()) {
        object.insert("warning".to_owned(), json!(warning));
    }

    Value::Object(object)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};
    use std::sync::Arc;

    use axum::extract::{Path, Query, State};
    use axum::response::IntoResponse;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use serde::Deserialize;
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use tokio::sync::{Mutex, Semaphore, broadcast};
    use tokio_util::sync::CancellationToken;

    use crate::server::execution::load_batch::{
        LoadTelemetryState, add_load_context_fields, apply_runner_telemetry_line,
        build_rps_history_sample, consolidate_load_metrics, drain_load_chunk,
        forward_runner_polled_load_chunked, lifecycle_curve_adherence,
        merge_runner_error_samples_into, rebuild_final_rps_history, rps_history_elapsed_bucket_ms,
        rps_history_timestamp, runner_load_poll_concurrency, snapshot_telemetry_map,
        summarize_load_latency, upsert_rps_history_samples,
    };
    use crate::server::execution::scheduler::SharedValue;
    use crate::server::models::{
        ConsolidatedLoadLifecycleBucket, ConsolidatedLoadMetrics, LoadEventContext,
        LoadLatencyAccumulator, LoadLatencySummary, NodePlan, RunnerLoadLine,
    };

    fn with_wave_lag_metrics(
        mut payload: Value,
        sender_start_lag: (f64, u64, u64, u64),
        http_send_duration: (f64, u64, u64),
        response_observation_duration: (f64, u64, u64),
    ) -> Value {
        let object = payload
            .as_object_mut()
            .expect("payload should be an object");
        object.insert("senderStartLagAvgMs".to_owned(), json!(sender_start_lag.0));
        object.insert("senderStartLagP95Ms".to_owned(), json!(sender_start_lag.1));
        object.insert("senderStartLagP99Ms".to_owned(), json!(sender_start_lag.2));
        object.insert("senderStartLagMaxMs".to_owned(), json!(sender_start_lag.3));
        object.insert(
            "httpSendDurationAvgMs".to_owned(),
            json!(http_send_duration.0),
        );
        object.insert(
            "httpSendDurationP95Ms".to_owned(),
            json!(http_send_duration.1),
        );
        object.insert(
            "httpSendDurationP99Ms".to_owned(),
            json!(http_send_duration.2),
        );
        object.insert(
            "responseObservationDurationAvgMs".to_owned(),
            json!(response_observation_duration.0),
        );
        object.insert(
            "responseObservationDurationP95Ms".to_owned(),
            json!(response_observation_duration.1),
        );
        object.insert(
            "responseObservationDurationP99Ms".to_owned(),
            json!(response_observation_duration.2),
        );
        payload
    }

    #[derive(Clone)]
    struct PollingRunnerState {
        acked: Arc<Mutex<Vec<u64>>>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct AckRequest {
        through_seq: u64,
    }

    async fn mock_load_start(Json(_payload): Json<Value>) -> impl IntoResponse {
        Json(json!({
            "runnerExecutionId": "runner-exec-1",
            "status": "running",
            "nextSeq": 1,
            "startedAtMs": 1
        }))
    }

    async fn mock_load_telemetry(
        Path(_execution_id): Path<String>,
        Query(query): Query<HashMap<String, String>>,
    ) -> impl IntoResponse {
        let after_seq = query
            .get("afterSeq")
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        if after_seq >= 2 {
            return Json(json!({
                "runnerExecutionId": "runner-exec-1",
                "status": "completed",
                "fromSeq": after_seq,
                "throughSeq": after_seq,
                "nextSeq": after_seq + 1,
                "buckets": []
            }));
        }

        Json(json!({
            "runnerExecutionId": "runner-exec-1",
            "status": "completed",
            "fromSeq": 0,
            "throughSeq": 2,
            "nextSeq": 3,
            "buckets": [
                {
                    "seq": 1,
                    "event": "execution:init",
                    "elapsedMs": 0,
                    "payload": { "executionId": "runner-exec-1" }
                },
                {
                    "seq": 2,
                    "event": "metrics",
                    "elapsedMs": 1000,
                    "payload": {
                        "totalStarted": 2,
                        "totalSent": 2,
                        "totalSuccess": 2,
                        "totalError": 0,
                        "httpStarted": 2,
                        "httpCompleted": 2,
                        "rps": 2.0,
                        "startTime": 1,
                        "elapsedMs": 1000,
                        "durationMs": 10
                    }
                }
            ]
        }))
    }

    async fn mock_load_ack(
        State(state): State<PollingRunnerState>,
        Path(_execution_id): Path<String>,
        Json(payload): Json<AckRequest>,
    ) -> impl IntoResponse {
        state.acked.lock().await.push(payload.through_seq);
        Json(json!({
            "runnerExecutionId": "runner-exec-1",
            "ackedThroughSeq": payload.through_seq,
            "retainedFromSeq": payload.through_seq + 1
        }))
    }

    async fn mock_load_cancel(Path(execution_id): Path<String>) -> impl IntoResponse {
        Json(json!({ "runnerExecutionId": execution_id, "status": "cancelled" }))
    }

    async fn spawn_polling_runner() -> (String, Arc<Mutex<Vec<u64>>>) {
        let acked = Arc::new(Mutex::new(Vec::new()));
        let state = PollingRunnerState {
            acked: Arc::clone(&acked),
        };
        let app = Router::new()
            .route("/api/v1/tests/load/start", post(mock_load_start))
            .route(
                "/api/v1/tests/load/{execution_id}/telemetry",
                get(mock_load_telemetry),
            )
            .route(
                "/api/v1/tests/load/{execution_id}/telemetry/ack",
                post(mock_load_ack),
            )
            .route(
                "/api/v1/tests/load/{execution_id}/cancel",
                post(mock_load_cancel),
            )
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{}", addr), acked)
    }

    #[tokio::test]
    async fn polled_load_forwarder_collects_metrics_and_acks_runner_buckets() {
        let (node, acked) = spawn_polling_runner().await;
        let client = reqwest::Client::new();
        let (tx, _rx) = broadcast::channel(16);
        let load_chunk = Arc::new(Mutex::new(HashMap::new()));
        let load_telemetry = Arc::new(Mutex::new(LoadTelemetryState::default()));
        let load_latency = Arc::new(Mutex::new(LoadLatencyAccumulator::default()));
        let load_errors = Arc::new(Mutex::new(Vec::new()));
        let load_context = Arc::new(LoadEventContext {
            plan: NodePlan {
                requested_nodes: 1,
                nodes_found: 1,
                nodes_used: 1,
                warning: None,
            },
            warning: None,
            registered_nodes: vec![node.clone()],
            active_nodes: vec![node.clone()],
            used_nodes: vec![node.clone()],
            runner_load_plan: Vec::new(),
            batch_window_ms: 100,
        });
        let snapshot_payload = SharedValue::new(json!({}));

        forward_runner_polled_load_chunked(
            &client,
            node.clone(),
            json!({ "config": { "totalRequests": 1 } }),
            tx,
            CancellationToken::new(),
            Arc::clone(&load_chunk),
            Arc::clone(&load_telemetry),
            Arc::clone(&load_latency),
            Arc::clone(&load_errors),
            load_context,
            "load-exec-1".to_owned(),
            snapshot_payload,
            "/api/v1/tests/load/start",
            None,
            None,
            None,
            Arc::new(Semaphore::new(runner_load_poll_concurrency())),
        )
        .await;

        let lines = drain_load_chunk(&load_chunk).await;
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].runner_event, "metrics");
        assert_eq!(lines[0].payload["totalSent"], json!(2));
        assert_eq!(*acked.lock().await, vec![2]);
        assert!(load_errors.lock().await.is_empty());
    }

    #[test]
    fn load_context_fields_include_registered_and_active_nodes() {
        let plan = NodePlan {
            requested_nodes: 2,
            nodes_found: 2,
            nodes_used: 2,
            warning: Some("warn".to_owned()),
        };
        let context = LoadEventContext {
            plan,
            warning: Some("warn".to_owned()),
            registered_nodes: vec![
                "http://runner-a:3000".to_owned(),
                "http://runner-b:3000".to_owned(),
                "http://runner-c:3000".to_owned(),
            ],
            active_nodes: vec![
                "http://runner-a:3000".to_owned(),
                "http://runner-b:3000".to_owned(),
            ],
            used_nodes: vec![
                "http://runner-a:3000".to_owned(),
                "http://runner-b:3000".to_owned(),
            ],
            runner_load_plan: vec![
                crate::server::models::RunnerLoadPlanItem {
                    node: "http://runner-a:3000".to_owned(),
                    total_requests: 60,
                    concurrency: 30,
                    desired_total_requests: 50,
                    above_desired: true,
                },
                crate::server::models::RunnerLoadPlanItem {
                    node: "http://runner-b:3000".to_owned(),
                    total_requests: 40,
                    concurrency: 20,
                    desired_total_requests: 50,
                    above_desired: false,
                },
            ],
            batch_window_ms: 50,
        };

        let payload = add_load_context_fields(json!({ "x": 1 }), &context);
        assert_eq!(payload["registeredNodesTotal"], json!(3));
        assert_eq!(payload["activeNodesTotal"], json!(2));
        assert_eq!(payload["usedNodesTotal"], json!(2));
        assert_eq!(payload["batchWindowMs"], json!(50));
        assert_eq!(payload["warning"], json!("warn"));
        assert_eq!(
            payload["runnerLoadPlan"],
            json!([
                {
                    "node": "http://runner-a:3000",
                    "totalRequests": 60,
                    "concurrency": 30,
                    "desiredTotalRequests": 50,
                    "aboveDesired": true
                },
                {
                    "node": "http://runner-b:3000",
                    "totalRequests": 40,
                    "concurrency": 20,
                    "desiredTotalRequests": 50,
                    "aboveDesired": false
                }
            ])
        );
        assert_eq!(
            payload["registeredNodes"],
            json!([
                "http://runner-a:3000",
                "http://runner-b:3000",
                "http://runner-c:3000"
            ])
        );
    }

    #[test]
    fn consolidates_latest_metrics_from_all_nodes() {
        let latest = HashMap::from([
            (
                "http://runner-a:3000".to_owned(),
                RunnerLoadLine {
                    node: "http://runner-a:3000".to_owned(),
                    runner_event: "metrics".to_owned(),
                    received_at: 1,
                    payload: json!({
                        "totalSent": 100,
                        "totalStarted": 120,
                        "totalSuccess": 90,
                        "totalError": 10,
                        "rps": 50.5,
                        "startTime": 1_000,
                        "elapsedMs": 8_000
                    }),
                },
            ),
            (
                "http://runner-b:3000".to_owned(),
                RunnerLoadLine {
                    node: "http://runner-b:3000".to_owned(),
                    runner_event: "metrics".to_owned(),
                    received_at: 2,
                    payload: json!({
                        "totalSent": 70,
                        "totalStarted": 80,
                        "totalSuccess": 70,
                        "totalError": 0,
                        "rps": 30.0,
                        "startTime": 900,
                        "elapsedMs": 9_500
                    }),
                },
            ),
        ]);

        let mut latency = LoadLatencyAccumulator::default();
        for duration_ms in [100u64, 200, 300, 400, 500] {
            latency.add_sample(duration_ms);
        }
        let latency_summary = summarize_load_latency(&latency);

        let consolidated =
            consolidate_load_metrics(&latest, latency_summary).expect("expected consolidated data");
        assert_eq!(consolidated.total_started, Some(200));
        assert_eq!(consolidated.total_sent, 170);
        assert_eq!(consolidated.total_success, 160);
        assert_eq!(consolidated.total_error, 10);
        assert!((consolidated.rps - 80.5).abs() < f64::EPSILON);
        assert_eq!(consolidated.avg_latency, 300);
        assert_eq!(consolidated.p95, 500);
        assert_eq!(consolidated.p99, 500);
        assert_eq!(consolidated.start_time, 900);
        assert_eq!(consolidated.elapsed_ms, 9_500);
        assert_eq!(consolidated.nodes_reporting, 2);
    }

    #[test]
    fn consolidates_with_zero_latency_when_no_samples_exist() {
        let latest = HashMap::from([(
            "http://runner-a:3000".to_owned(),
            RunnerLoadLine {
                node: "http://runner-a:3000".to_owned(),
                runner_event: "metrics".to_owned(),
                received_at: 1,
                payload: json!({
                    "totalSent": 10,
                    "totalSuccess": 9,
                    "totalError": 1,
                    "rps": 12.5,
                    "startTime": 1_000,
                    "elapsedMs": 2_000
                }),
            },
        )]);

        let consolidated = consolidate_load_metrics(&latest, LoadLatencySummary::default())
            .expect("expected consolidated data");
        assert_eq!(consolidated.avg_latency, 0);
        assert_eq!(consolidated.p95, 0);
        assert_eq!(consolidated.p99, 0);
    }

    #[test]
    fn consolidates_latency_from_runner_histograms() {
        let latest = HashMap::from([
            (
                "http://runner-a:3000".to_owned(),
                RunnerLoadLine {
                    node: "http://runner-a:3000".to_owned(),
                    runner_event: "metrics".to_owned(),
                    received_at: 1,
                    payload: json!({
                        "totalSent": 2,
                        "totalSuccess": 1,
                        "totalError": 1,
                        "rps": 10.0,
                        "startTime": 1_000,
                        "elapsedMs": 1_000,
                        "latencySampleCount": 2,
                        "latencyTotalDurationMs": 300,
                        "latencyBuckets": [
                            { "durationMs": 100, "count": 1 },
                            { "durationMs": 200, "count": 1 }
                        ]
                    }),
                },
            ),
            (
                "http://runner-b:3000".to_owned(),
                RunnerLoadLine {
                    node: "http://runner-b:3000".to_owned(),
                    runner_event: "metrics".to_owned(),
                    received_at: 1,
                    payload: json!({
                        "totalSent": 2,
                        "totalSuccess": 0,
                        "totalError": 2,
                        "rps": 20.0,
                        "startTime": 900,
                        "elapsedMs": 1_200,
                        "latencySampleCount": 2,
                        "latencyTotalDurationMs": 100,
                        "latencyBuckets": [
                            { "durationMs": 50, "count": 2 }
                        ]
                    }),
                },
            ),
        ]);

        let consolidated = consolidate_load_metrics(&latest, LoadLatencySummary::default())
            .expect("expected consolidated metrics");

        assert_eq!(consolidated.total_sent, 4);
        assert_eq!(consolidated.avg_latency, 100);
        assert_eq!(consolidated.p95, 200);
        assert_eq!(consolidated.p99, 200);
    }

    #[test]
    fn consolidates_dispatch_metrics() {
        let latest = HashMap::from([
            (
                "http://runner-a:3000".to_owned(),
                RunnerLoadLine {
                    node: "http://runner-a:3000".to_owned(),
                    runner_event: "metrics".to_owned(),
                    received_at: 1,
                    payload: with_wave_lag_metrics(
                        json!({
                            "totalSent": 10,
                            "totalSuccess": 8,
                            "totalError": 2,
                            "rps": 100.0,
                            "startTime": 1_000,
                            "elapsedMs": 2_000,
                            "scheduledStarts": 100,
                            "missedStarts": 5,
                            "dispatchSubmitted": 100,
                            "dispatchStarted": 95,
                            "httpSendReturned": 80,
                            "responseBodyCompleted": 70,
                            "dependencyLimitedStarts": 1,
                            "dispatcherLaggedStarts": 5,
                            "runtimeLaggedStarts": 2,
                            "senderLaggedStarts": 3,
                            "senderQueueDepth": 11,
                            "schedulerLagMs": 30,
                            "schedulerLaggedStarts": 4,
                            "slotEnqueued": 100,
                            "requestPrepared": 99,
                            "requestEnqueued": 98,
                            "sendTaskSpawned": 97,
                            "sendStarted": 96,
                            "readyRequests": 20,
                            "activePipelines": 50,
                            "outstandingRequests": 30,
                            "curveAdherence": 95.0,
                            "lifecycleBuckets": [
                                {"elapsedMs": 1_000, "planned": 10, "sendStarted": 9, "httpStarted": 7, "senderLagged": 2, "senderStartLagMsMax": 11, "httpSendDurationMsMax": 22, "responseObservationDurationMsMax": 33}
                            ]
                        }),
                        (10.0, 20, 30, 40),
                        (50.0, 60, 70),
                        (80.0, 90, 100),
                    ),
                },
            ),
            (
                "http://runner-b:3000".to_owned(),
                RunnerLoadLine {
                    node: "http://runner-b:3000".to_owned(),
                    runner_event: "metrics".to_owned(),
                    received_at: 1,
                    payload: with_wave_lag_metrics(
                        json!({
                            "totalSent": 20,
                            "totalSuccess": 19,
                            "totalError": 1,
                            "rps": 120.0,
                            "startTime": 900,
                            "elapsedMs": 2_100,
                            "scheduledStarts": 100,
                            "missedStarts": 15,
                            "dispatchSubmitted": 100,
                            "dispatchStarted": 85,
                            "httpSendReturned": 90,
                            "responseBodyCompleted": 85,
                            "dependencyLimitedStarts": 3,
                            "dispatcherLaggedStarts": 7,
                            "runtimeLaggedStarts": 4,
                            "senderLaggedStarts": 9,
                            "senderQueueDepth": 13,
                            "schedulerLagMs": 50,
                            "schedulerLaggedStarts": 6,
                            "slotEnqueued": 100,
                            "requestPrepared": 98,
                            "requestEnqueued": 97,
                            "sendTaskSpawned": 96,
                            "sendStarted": 95,
                            "readyRequests": 30,
                            "activePipelines": 60,
                            "outstandingRequests": 40,
                            "curveAdherence": 85.0,
                            "lifecycleBuckets": [
                                {"elapsedMs": 1_000, "planned": 20, "sendStarted": 20, "httpStarted": 14, "senderLagged": 5, "senderStartLagMsMax": 44, "httpSendDurationMsMax": 55, "responseObservationDurationMsMax": 66}
                            ]
                        }),
                        (15.0, 25, 35, 45),
                        (55.0, 65, 75),
                        (85.0, 95, 105),
                    ),
                },
            ),
        ]);

        let consolidated = consolidate_load_metrics(&latest, LoadLatencySummary::default())
            .expect("expected consolidated data");

        assert_eq!(consolidated.scheduled_starts, Some(200));
        assert_eq!(consolidated.missed_starts, Some(20));
        assert_eq!(consolidated.dispatch_submitted, Some(200));
        assert_eq!(consolidated.dispatch_started, Some(180));
        assert_eq!(consolidated.http_send_returned, Some(170));
        assert_eq!(consolidated.response_body_completed, Some(155));
        assert_eq!(consolidated.dependency_limited_starts, Some(4));
        assert_eq!(consolidated.dispatcher_lagged_starts, Some(12));
        assert_eq!(consolidated.runtime_lagged_starts, Some(6));
        assert_eq!(consolidated.sender_lagged_starts, Some(12));
        assert_eq!(consolidated.sender_queue_depth, Some(24));
        assert_eq!(consolidated.sender_start_lag_avg_ms, Some(15.0));
        assert_eq!(consolidated.sender_start_lag_p95_ms, Some(25));
        assert_eq!(consolidated.sender_start_lag_p99_ms, Some(35));
        assert_eq!(consolidated.sender_start_lag_max_ms, Some(45));
        assert_eq!(consolidated.http_send_duration_avg_ms, Some(55.0));
        assert_eq!(consolidated.http_send_duration_p95_ms, Some(65));
        assert_eq!(consolidated.http_send_duration_p99_ms, Some(75));
        assert_eq!(
            consolidated.response_observation_duration_avg_ms,
            Some(85.0)
        );
        assert_eq!(consolidated.response_observation_duration_p95_ms, Some(95));
        assert_eq!(consolidated.response_observation_duration_p99_ms, Some(105));
        assert_eq!(consolidated.scheduler_lag_ms, Some(80));
        assert_eq!(consolidated.scheduler_lagged_starts, Some(10));
        assert_eq!(consolidated.slot_enqueued, Some(200));
        assert_eq!(consolidated.request_prepared, Some(197));
        assert_eq!(consolidated.request_enqueued, Some(195));
        assert_eq!(consolidated.send_task_spawned, Some(193));
        assert_eq!(consolidated.send_started, Some(191));
        assert_eq!(consolidated.ready_requests, Some(50));
        assert_eq!(consolidated.active_pipelines, Some(110));
        assert_eq!(consolidated.outstanding_requests, Some(70));
        assert_eq!(consolidated.lifecycle_buckets[0].sender_lagged, 7);
        assert_eq!(
            consolidated.lifecycle_buckets[0].sender_start_lag_ms_max,
            44
        );
        assert_eq!(
            consolidated.lifecycle_buckets[0].http_send_duration_ms_max,
            55
        );
        assert_eq!(
            consolidated.lifecycle_buckets[0].response_observation_duration_ms_max,
            66
        );
        assert_eq!(consolidated.curve_adherence, Some(96.67));
    }

    #[test]
    fn curve_adherence_uses_send_started_not_http_started() {
        let lifecycle = BTreeMap::from([(
            1_000,
            ConsolidatedLoadLifecycleBucket {
                elapsed_ms: 1_000,
                planned: 100,
                slot_enqueued: 0,
                request_prepared: 0,
                request_enqueued: 0,
                send_task_spawned: 0,
                send_started: 100,
                http_started: 60,
                http_send_returned: 0,
                response_body_completed: 0,
                dispatcher_lagged: 0,
                runtime_lagged: 0,
                sender_lagged: 0,
                sender_start_lag_ms_max: 0,
                http_send_duration_ms_max: 0,
                response_observation_duration_ms_max: 0,
            },
        )]);

        assert_eq!(lifecycle_curve_adherence(&lifecycle), Some(100.0));
    }

    #[test]
    fn merges_runner_error_samples_with_deduped_counts() {
        let mut errors = vec!["existing runner error".to_owned()];
        let payload = json!({
            "errorSamples": [
                {
                    "stepId": "create_user",
                    "httpStatus": 409,
                    "error": "HTTP 409 Conflict",
                    "count": 2
                },
                {
                    "stepId": "get_created_user",
                    "httpStatus": 404,
                    "error": "HTTP 404 Not Found",
                    "count": 1
                }
            ]
        });

        merge_runner_error_samples_into(&mut errors, "http://runner-a:3000", &payload);
        merge_runner_error_samples_into(
            &mut errors,
            "http://runner-a:3000",
            &json!({
                "errorSamples": [{
                    "stepId": "create_user",
                    "httpStatus": 409,
                    "error": "HTTP 409 Conflict",
                    "count": 5
                }]
            }),
        );

        assert_eq!(
            errors,
            vec![
                "existing runner error".to_owned(),
                "http://runner-a:3000 create_user HTTP 409: HTTP 409 Conflict (x5)".to_owned(),
                "http://runner-a:3000 get_created_user HTTP 404: HTTP 404 Not Found (x1)"
                    .to_owned(),
            ]
        );
    }

    #[test]
    fn consolidated_metrics_sum_lifecycle_buckets_by_elapsed_ms() {
        let mut latest = HashMap::new();
        latest.insert(
            "runner-a".to_owned(),
            RunnerLoadLine {
                node: "runner-a".to_owned(),
                runner_event: "metrics".to_owned(),
                received_at: 1,
                payload: json!({
                    "totalSent": 0,
                    "totalSuccess": 0,
                    "totalError": 0,
                    "rps": 0.0,
                    "startTime": 10_000,
                    "elapsedMs": 2_000,
                    "lifecycleBuckets": [
                        {"elapsedMs": 1_000, "planned": 10, "sendStarted": 9, "httpStarted": 8}
                    ]
                }),
            },
        );
        latest.insert(
            "runner-b".to_owned(),
            RunnerLoadLine {
                node: "runner-b".to_owned(),
                runner_event: "metrics".to_owned(),
                received_at: 1,
                payload: json!({
                    "totalSent": 0,
                    "totalSuccess": 0,
                    "totalError": 0,
                    "rps": 0.0,
                    "startTime": 10_000,
                    "elapsedMs": 2_000,
                    "lifecycleBuckets": [
                        {"elapsedMs": 1_000, "planned": 7, "sendStarted": 6, "httpStarted": 5}
                    ]
                }),
            },
        );

        let metrics = consolidate_load_metrics(&latest, LoadLatencySummary::default()).unwrap();

        assert_eq!(metrics.lifecycle_buckets.len(), 1);
        assert_eq!(metrics.lifecycle_buckets[0].elapsed_ms, 1_000);
        assert_eq!(metrics.lifecycle_buckets[0].planned, 17);
        assert_eq!(metrics.lifecycle_buckets[0].send_started, 15);
        assert_eq!(metrics.lifecycle_buckets[0].http_started, 13);
    }

    #[test]
    fn consolidated_metrics_sum_status_code_buckets_by_elapsed_and_code() {
        let mut latest = HashMap::new();
        latest.insert(
            "runner-a".to_owned(),
            RunnerLoadLine {
                node: "runner-a".to_owned(),
                runner_event: "metrics".to_owned(),
                received_at: 1,
                payload: json!({
                    "totalSent": 0,
                    "totalSuccess": 0,
                    "totalError": 0,
                    "rps": 0.0,
                    "startTime": 10_000,
                    "elapsedMs": 2_000,
                    "statusCodeBuckets": [
                        {"elapsedMs": 1_000, "code": "200", "count": 10},
                        {"elapsedMs": 1_000, "code": "502", "count": 2}
                    ]
                }),
            },
        );
        latest.insert(
            "runner-b".to_owned(),
            RunnerLoadLine {
                node: "runner-b".to_owned(),
                runner_event: "metrics".to_owned(),
                received_at: 1,
                payload: json!({
                    "totalSent": 0,
                    "totalSuccess": 0,
                    "totalError": 0,
                    "rps": 0.0,
                    "startTime": 10_000,
                    "elapsedMs": 2_000,
                    "statusCodeBuckets": [
                        {"elapsedMs": 1_000, "code": "200", "count": 7},
                        {"elapsedMs": 2_000, "code": "network_error", "count": 1}
                    ]
                }),
            },
        );

        let metrics = consolidate_load_metrics(&latest, LoadLatencySummary::default()).unwrap();

        assert_eq!(metrics.status_code_buckets.len(), 3);
        assert_eq!(metrics.status_code_buckets[0].elapsed_ms, 1_000);
        assert_eq!(metrics.status_code_buckets[0].code, "200");
        assert_eq!(metrics.status_code_buckets[0].count, 17);
        assert_eq!(metrics.status_code_buckets[1].elapsed_ms, 1_000);
        assert_eq!(metrics.status_code_buckets[1].code, "502");
        assert_eq!(metrics.status_code_buckets[1].count, 2);
        assert_eq!(metrics.status_code_buckets[2].elapsed_ms, 2_000);
        assert_eq!(metrics.status_code_buckets[2].code, "network_error");
        assert_eq!(metrics.status_code_buckets[2].count, 1);
    }

    #[test]
    fn builds_rps_history_sample_with_wave_targets() {
        let metrics = ConsolidatedLoadMetrics {
            total_started: Some(45),
            total_sent: 42,
            total_success: 40,
            total_error: 2,
            http_started: Some(60),
            http_completed: Some(58),
            dispatch_submitted: None,
            dispatch_started: None,
            http_send_returned: None,
            response_body_completed: None,
            dependency_limited_starts: None,
            dispatcher_lagged_starts: None,
            runtime_lagged_starts: None,
            sender_lagged_starts: None,
            sender_queue_depth: None,
            sender_start_lag_avg_ms: None,
            sender_start_lag_p95_ms: None,
            sender_start_lag_p99_ms: None,
            sender_start_lag_max_ms: None,
            http_send_duration_avg_ms: None,
            http_send_duration_p95_ms: None,
            http_send_duration_p99_ms: None,
            response_observation_duration_avg_ms: None,
            response_observation_duration_p95_ms: None,
            response_observation_duration_p99_ms: None,
            scheduler_lag_ms: None,
            scheduler_lagged_starts: None,
            slot_enqueued: None,
            request_prepared: None,
            request_enqueued: None,
            send_task_spawned: None,
            send_started: None,
            rps: 21.5,
            target_intensity: Some(75.0),
            target_rps_limit: Some(150.0),
            in_flight: Some(3),
            runner_max_rps: Some(200.0),
            tick_ms: Some(500),
            scheduled_starts: None,
            missed_starts: None,
            ready_requests: None,
            active_pipelines: None,
            outstanding_requests: None,
            curve_adherence: None,
            avg_latency: 10,
            p95: 20,
            p99: 30,
            start_time: 1_000,
            elapsed_ms: 2_450,
            nodes_reporting: 2,
            lifecycle_buckets: Vec::new(),
            status_code_buckets: Vec::new(),
        };
        let latest = HashMap::from([(
            "http://runner-a:3000".to_owned(),
            RunnerLoadLine {
                node: "http://runner-a:3000".to_owned(),
                runner_event: "metrics".to_owned(),
                received_at: 1,
                payload: json!({
                    "totalSent": 42,
                    "totalStarted": 45,
                    "totalSuccess": 40,
                    "totalError": 2,
                    "httpStarted": 60,
                    "httpCompleted": 58,
                    "dispatchBuckets": [
                        { "elapsedMs": 1_000, "count": 20 },
                        { "elapsedMs": 2_000, "count": 40 }
                    ],
                    "rps": 21.5,
                    "startTime": 1_500,
                    "elapsedMs": 2_000
                }),
            },
        )]);

        assert_eq!(rps_history_elapsed_bucket_ms(2_450), 1_000);
        assert_eq!(rps_history_timestamp(&metrics), Some(2_000));
        assert_eq!(
            build_rps_history_sample(rps_history_timestamp(&metrics).unwrap(), &metrics, &latest),
            json!({
                "timestamp": 2_000,
                "elapsedMs": 1_000,
                "rps": 21.5,
                "totalStarted": 45,
                "totalSent": 42,
                "httpStarted": 60,
                "httpCompleted": 58,
                "dispatchBucket": 20,
                "targetIntensity": 75.0,
                "targetRpsLimit": 150.0,
                "runners": [{
                    "runnerId": "http://runner-a:3000",
                    "httpStarted": 60,
                    "httpCompleted": 58,
                    "totalStarted": 45,
                    "totalSent": 42,
                    "dispatchBucket": 20,
                    "rps": 21.5
                }]
            })
        );
    }

    #[test]
    fn upserts_recent_rps_history_buckets_when_runner_snapshots_become_complete() {
        let metrics = ConsolidatedLoadMetrics {
            total_started: Some(45),
            total_sent: 42,
            total_success: 40,
            total_error: 2,
            http_started: Some(60),
            http_completed: Some(58),
            dispatch_submitted: None,
            dispatch_started: None,
            http_send_returned: None,
            response_body_completed: None,
            dependency_limited_starts: None,
            dispatcher_lagged_starts: None,
            runtime_lagged_starts: None,
            sender_lagged_starts: None,
            sender_queue_depth: None,
            sender_start_lag_avg_ms: None,
            sender_start_lag_p95_ms: None,
            sender_start_lag_p99_ms: None,
            sender_start_lag_max_ms: None,
            http_send_duration_avg_ms: None,
            http_send_duration_p95_ms: None,
            http_send_duration_p99_ms: None,
            response_observation_duration_avg_ms: None,
            response_observation_duration_p95_ms: None,
            response_observation_duration_p99_ms: None,
            scheduler_lag_ms: None,
            scheduler_lagged_starts: None,
            slot_enqueued: None,
            request_prepared: None,
            request_enqueued: None,
            send_task_spawned: None,
            send_started: None,
            rps: 21.5,
            target_intensity: Some(75.0),
            target_rps_limit: Some(150.0),
            in_flight: Some(3),
            runner_max_rps: Some(200.0),
            tick_ms: Some(500),
            scheduled_starts: None,
            missed_starts: None,
            ready_requests: None,
            active_pipelines: None,
            outstanding_requests: None,
            curve_adherence: None,
            avg_latency: 10,
            p95: 20,
            p99: 30,
            start_time: 1_000,
            elapsed_ms: 4_250,
            nodes_reporting: 2,
            lifecycle_buckets: Vec::new(),
            status_code_buckets: Vec::new(),
        };
        let mut history = BTreeMap::new();
        let partial = HashMap::from([(
            "http://runner-a:3000".to_owned(),
            RunnerLoadLine {
                node: "http://runner-a:3000".to_owned(),
                runner_event: "metrics".to_owned(),
                received_at: 1,
                payload: json!({
                    "totalSent": 42,
                    "totalSuccess": 40,
                    "totalError": 2,
                    "rps": 21.5,
                    "startTime": 1_000,
                    "elapsedMs": 4_250,
                    "dispatchBuckets": [{ "elapsedMs": 2_000, "count": 10 }]
                }),
            },
        )]);
        let complete = HashMap::from([(
            "http://runner-a:3000".to_owned(),
            RunnerLoadLine {
                node: "http://runner-a:3000".to_owned(),
                runner_event: "metrics".to_owned(),
                received_at: 2,
                payload: json!({
                    "totalSent": 42,
                    "totalSuccess": 40,
                    "totalError": 2,
                    "rps": 21.5,
                    "startTime": 1_000,
                    "elapsedMs": 4_250,
                    "dispatchBuckets": [{ "elapsedMs": 2_000, "count": 25 }]
                }),
            },
        )]);

        upsert_rps_history_samples(&mut history, &metrics, &partial);
        upsert_rps_history_samples(&mut history, &metrics, &complete);

        let sample = history
            .get(&3_000)
            .and_then(Value::as_object)
            .expect("bucket should exist");
        assert_eq!(
            sample.get("dispatchBucket").and_then(Value::as_u64),
            Some(25)
        );
    }

    #[test]
    fn rebuilds_final_rps_history_from_runner_dispatch_buckets() {
        let metrics = ConsolidatedLoadMetrics {
            total_started: Some(45),
            total_sent: 42,
            total_success: 40,
            total_error: 2,
            http_started: Some(60),
            http_completed: Some(58),
            dispatch_submitted: None,
            dispatch_started: Some(35),
            http_send_returned: None,
            response_body_completed: None,
            dependency_limited_starts: None,
            dispatcher_lagged_starts: None,
            runtime_lagged_starts: None,
            sender_lagged_starts: None,
            sender_queue_depth: None,
            sender_start_lag_avg_ms: None,
            sender_start_lag_p95_ms: None,
            sender_start_lag_p99_ms: None,
            sender_start_lag_max_ms: None,
            http_send_duration_avg_ms: None,
            http_send_duration_p95_ms: None,
            http_send_duration_p99_ms: None,
            response_observation_duration_avg_ms: None,
            response_observation_duration_p95_ms: None,
            response_observation_duration_p99_ms: None,
            scheduler_lag_ms: None,
            scheduler_lagged_starts: None,
            slot_enqueued: None,
            request_prepared: None,
            request_enqueued: None,
            send_task_spawned: None,
            send_started: None,
            rps: 21.5,
            target_intensity: Some(75.0),
            target_rps_limit: Some(150.0),
            in_flight: Some(3),
            runner_max_rps: Some(200.0),
            tick_ms: Some(500),
            scheduled_starts: None,
            missed_starts: None,
            ready_requests: None,
            active_pipelines: None,
            outstanding_requests: None,
            curve_adherence: None,
            avg_latency: 10,
            p95: 20,
            p99: 30,
            start_time: 1_000,
            elapsed_ms: 4_250,
            nodes_reporting: 2,
            lifecycle_buckets: Vec::new(),
            status_code_buckets: Vec::new(),
        };
        let latest = HashMap::from([
            (
                "http://runner-a:3000".to_owned(),
                RunnerLoadLine {
                    node: "http://runner-a:3000".to_owned(),
                    runner_event: "metrics".to_owned(),
                    received_at: 1,
                    payload: json!({
                        "totalSent": 42,
                        "totalSuccess": 40,
                        "totalError": 2,
                        "rps": 21.5,
                        "startTime": 1_000,
                        "elapsedMs": 4_250,
                        "dispatchBuckets": [
                            { "elapsedMs": 0, "count": 10 },
                            { "elapsedMs": 1_000, "count": 20 }
                        ]
                    }),
                },
            ),
            (
                "http://runner-b:3000".to_owned(),
                RunnerLoadLine {
                    node: "http://runner-b:3000".to_owned(),
                    runner_event: "metrics".to_owned(),
                    received_at: 1,
                    payload: json!({
                        "totalSent": 42,
                        "totalSuccess": 40,
                        "totalError": 2,
                        "rps": 21.5,
                        "startTime": 1_000,
                        "elapsedMs": 4_250,
                        "dispatchBuckets": [
                            { "elapsedMs": 0, "count": 5 },
                            { "elapsedMs": 1_000, "count": 15 }
                        ]
                    }),
                },
            ),
        ]);

        let history = rebuild_final_rps_history(&metrics, &latest);

        assert_eq!(history.len(), 2);
        assert_eq!(
            history[0].get("dispatchBucket").and_then(Value::as_u64),
            Some(15)
        );
        assert_eq!(
            history[1].get("dispatchBucket").and_then(Value::as_u64),
            Some(35)
        );
    }

    #[test]
    fn summarizes_latency_percentiles_from_global_histogram() {
        let mut latency = LoadLatencyAccumulator::default();
        // Simulates interleaved samples coming from multiple nodes.
        for duration_ms in [20u64, 300, 40, 200, 50, 100] {
            latency.add_sample(duration_ms);
        }

        let summary = summarize_load_latency(&latency);
        assert_eq!(summary.avg_latency, 118);
        assert_eq!(summary.p95, 300);
        assert_eq!(summary.p99, 300);
    }

    #[tokio::test]
    async fn drain_load_chunk_keeps_only_latest_line_per_node() {
        let chunk: Arc<Mutex<HashMap<String, RunnerLoadLine>>> =
            Arc::new(Mutex::new(HashMap::new()));

        {
            let mut lock = chunk.lock().await;
            lock.insert(
                "http://runner-a:3000".to_owned(),
                RunnerLoadLine {
                    node: "http://runner-a:3000".to_owned(),
                    runner_event: "metrics".to_owned(),
                    received_at: 1,
                    payload: json!({ "value": 1 }),
                },
            );
            lock.insert(
                "http://runner-b:3000".to_owned(),
                RunnerLoadLine {
                    node: "http://runner-b:3000".to_owned(),
                    runner_event: "metrics".to_owned(),
                    received_at: 2,
                    payload: json!({ "value": 10 }),
                },
            );
            lock.insert(
                "http://runner-a:3000".to_owned(),
                RunnerLoadLine {
                    node: "http://runner-a:3000".to_owned(),
                    runner_event: "metrics".to_owned(),
                    received_at: 3,
                    payload: json!({ "value": 2 }),
                },
            );
        }

        let lines = drain_load_chunk(&chunk).await;
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].node, "http://runner-a:3000");
        assert_eq!(lines[0].payload["value"], json!(2));
        assert_eq!(lines[1].node, "http://runner-b:3000");
        assert_eq!(lines[1].payload["value"], json!(10));

        let second_read = drain_load_chunk(&chunk).await;
        assert!(second_read.is_empty());
    }

    #[tokio::test]
    async fn telemetry_state_merges_live_bucket_windows_per_runner() {
        let telemetry = Arc::new(Mutex::new(LoadTelemetryState::default()));
        {
            let mut lock = telemetry.lock().await;
            apply_runner_telemetry_line(
                &mut lock,
                RunnerLoadLine {
                    node: "http://runner-a:3000".to_owned(),
                    runner_event: "metrics".to_owned(),
                    received_at: 1,
                    payload: json!({
                        "snapshotMode": "live",
                        "totalSent": 10,
                        "totalSuccess": 10,
                        "totalError": 0,
                        "rps": 10.0,
                        "startTime": 1_000,
                        "elapsedMs": 1_100,
                        "lifecycleBuckets": [{ "elapsedMs": 0, "planned": 10, "sendStarted": 10, "httpStarted": 9 }]
                    }),
                },
            );
            apply_runner_telemetry_line(
                &mut lock,
                RunnerLoadLine {
                    node: "http://runner-a:3000".to_owned(),
                    runner_event: "metrics".to_owned(),
                    received_at: 2,
                    payload: json!({
                        "snapshotMode": "live",
                        "totalSent": 20,
                        "totalSuccess": 20,
                        "totalError": 0,
                        "rps": 20.0,
                        "startTime": 1_000,
                        "elapsedMs": 2_100,
                        "lifecycleBuckets": [{ "elapsedMs": 1_000, "planned": 20, "sendStarted": 19, "httpStarted": 18 }]
                    }),
                },
            );
        }

        let snapshot = snapshot_telemetry_map(&telemetry).await;
        let consolidated = consolidate_load_metrics(&snapshot, LoadLatencySummary::default())
            .expect("expected consolidated telemetry");

        assert_eq!(consolidated.lifecycle_buckets.len(), 2);
        assert_eq!(consolidated.lifecycle_buckets[0].http_started, 9);
        assert_eq!(consolidated.lifecycle_buckets[1].http_started, 18);
        assert_eq!(consolidated.curve_adherence, Some(96.67));
    }
}
