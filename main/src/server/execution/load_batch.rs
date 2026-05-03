use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use serde::Serialize;
use serde_json::{Map, Value, json};
use tokio::sync::{Mutex, broadcast};
use tokio_stream::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::server::execution::forward::{parse_sse_block, send_sse_best_effort};
use crate::server::execution::history_capture::{extract_error_message, push_load_error};
use crate::server::execution::runner_auth::apply_runner_auth;
use crate::server::execution::scheduler::SharedValue;
use crate::server::execution::snapshot::build_live_load_snapshot_payload;
use crate::server::models::{
    ConsolidatedLoadMetrics, LoadEventContext, LoadLatencyAccumulator, LoadLatencySummary,
    RunnerLoadLine,
};
use crate::server::state::TRANSACTION_ID_HEADER;
use crate::server::utils::{now_ms, parse_runner_duration_ms, parse_runner_load_metrics};

pub async fn forward_runner_stream_load_chunked(
    client: &Client,
    node: String,
    body: Value,
    tx: broadcast::Sender<crate::server::models::SseMessage>,
    cancel: CancellationToken,
    load_chunk: Arc<Mutex<HashMap<String, RunnerLoadLine>>>,
    load_latest: Arc<Mutex<HashMap<String, RunnerLoadLine>>>,
    load_latency: Arc<Mutex<LoadLatencyAccumulator>>,
    load_errors: Arc<Mutex<Vec<String>>>,
    load_context: Arc<LoadEventContext>,
    execution_id: String,
    snapshot_payload: SharedValue<Value>,
    endpoint_path: &str,
    transaction_id: Option<String>,
    runner_auth_key: Option<&str>,
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

    let response = match tokio::time::timeout(Duration::from_secs(10), request.json(&body).send())
        .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(err)) => {
            push_load_error(&load_errors, format!("runner request failed: {}", err)).await;
            refresh_load_snapshot(
                &execution_id,
                &snapshot_payload,
                &load_latest,
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
            refresh_load_snapshot(
                &execution_id,
                &snapshot_payload,
                &load_latest,
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
        refresh_load_snapshot(
            &execution_id,
            &snapshot_payload,
            &load_latest,
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
                refresh_load_snapshot(
                    &execution_id,
                    &snapshot_payload,
                    &load_latest,
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

            let mut latest_lock = load_latest.lock().await;
            latest_lock.insert(node.clone(), line);
            drop(latest_lock);
            refresh_load_snapshot(
                &execution_id,
                &snapshot_payload,
                &load_latest,
                &load_latency,
                &load_errors,
                load_context.as_ref(),
                "running",
            )
            .await;
        }
    }
}

pub async fn flush_load_batches(
    execution_id: String,
    tx: broadcast::Sender<crate::server::models::SseMessage>,
    cancel: CancellationToken,
    stop: CancellationToken,
    load_chunk: Arc<Mutex<HashMap<String, RunnerLoadLine>>>,
    load_latest: Arc<Mutex<HashMap<String, RunnerLoadLine>>>,
    load_latency: Arc<Mutex<LoadLatencyAccumulator>>,
    load_errors: Arc<Mutex<Vec<String>>>,
    load_context: Arc<LoadEventContext>,
    snapshot_payload: SharedValue<Value>,
    rps_history: Arc<Mutex<Vec<Value>>>,
) {
    let mut interval =
        tokio::time::interval(Duration::from_millis(load_context.batch_window_ms.max(1)));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_rps_history_sample_at = 0u64;

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

        let latest_snapshot = {
            let lock = load_latest.lock().await;
            lock.clone()
        };
        let consolidated = {
            let latency_summary = {
                let lock = load_latency.lock().await;
                summarize_load_latency(&lock)
            };
            consolidate_load_metrics(&latest_snapshot, latency_summary)
        };
        if let Some(metrics) = consolidated.as_ref() {
            let sample_at = now_ms();
            if sample_at.saturating_sub(last_rps_history_sample_at) >= 500 {
                rps_history.lock().await.push(build_rps_history_sample(
                    sample_at,
                    metrics,
                    &latest_snapshot,
                ));
                last_rps_history_sample_at = sample_at;
            }
        }
        let errors = load_errors.lock().await.clone();
        let latest_lines = snapshot_latest_lines(&load_latest).await;
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

fn build_rps_history_sample(
    timestamp: u64,
    metrics: &ConsolidatedLoadMetrics,
    latest_by_node: &HashMap<String, RunnerLoadLine>,
) -> Value {
    let mut runners = latest_by_node
        .values()
        .filter_map(|line| {
            let metrics = parse_runner_load_metrics(&line.payload)?;
            let mut runner = Map::new();
            runner.insert("runnerId".to_owned(), Value::String(line.node.clone()));
            insert_optional(&mut runner, "httpStarted", metrics.http_started);
            insert_optional(&mut runner, "httpCompleted", metrics.http_completed);
            insert_optional(&mut runner, "dispatchSubmitted", metrics.dispatch_submitted);
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
                "runtimeLaggedStarts",
                metrics.runtime_lagged_starts,
            );
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
    sample.insert("rps".to_owned(), json!(metrics.rps));
    insert_optional(&mut sample, "totalStarted", metrics.total_started);
    sample.insert("totalSent".to_owned(), json!(metrics.total_sent));
    insert_optional(&mut sample, "httpStarted", metrics.http_started);
    insert_optional(&mut sample, "httpCompleted", metrics.http_completed);
    insert_optional(&mut sample, "dispatchSubmitted", metrics.dispatch_submitted);
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
        "runtimeLaggedStarts",
        metrics.runtime_lagged_starts,
    );
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

fn insert_optional<T: Serialize>(map: &mut Map<String, Value>, key: &str, value: Option<T>) {
    if let Some(value) = value {
        map.insert(key.to_owned(), json!(value));
    }
}

async fn refresh_load_snapshot(
    execution_id: &str,
    snapshot_payload: &SharedValue<Value>,
    load_latest: &Arc<Mutex<HashMap<String, RunnerLoadLine>>>,
    load_latency: &Arc<Mutex<LoadLatencyAccumulator>>,
    load_errors: &Arc<Mutex<Vec<String>>>,
    load_context: &LoadEventContext,
    status: &str,
) {
    let lines = snapshot_latest_lines(load_latest).await;
    let consolidated = snapshot_consolidated_metrics(load_latest, load_latency).await;
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

pub async fn drain_load_chunk(
    load_chunk: &Arc<Mutex<HashMap<String, RunnerLoadLine>>>,
) -> Vec<RunnerLoadLine> {
    let mut lock = load_chunk.lock().await;
    let mut lines: Vec<RunnerLoadLine> = lock.drain().map(|(_, line)| line).collect();
    lines.sort_by(|a, b| a.node.cmp(&b.node));
    lines
}

pub async fn snapshot_latest_lines(
    load_latest: &Arc<Mutex<HashMap<String, RunnerLoadLine>>>,
) -> Vec<RunnerLoadLine> {
    let lock = load_latest.lock().await;
    let mut lines: Vec<RunnerLoadLine> = lock.values().cloned().collect();
    lines.sort_by(|a, b| a.node.cmp(&b.node));
    lines
}

pub async fn snapshot_consolidated_metrics(
    load_latest: &Arc<Mutex<HashMap<String, RunnerLoadLine>>>,
    load_latency: &Arc<Mutex<LoadLatencyAccumulator>>,
) -> Option<ConsolidatedLoadMetrics> {
    let latest_snapshot = {
        let lock = load_latest.lock().await;
        lock.clone()
    };
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
    let mut http_send_returned = 0usize;
    let mut http_send_returned_nodes = 0usize;
    let mut response_body_completed = 0usize;
    let mut response_body_completed_nodes = 0usize;
    let mut dependency_limited_starts = 0usize;
    let mut dependency_limited_starts_nodes = 0usize;
    let mut runtime_lagged_starts = 0usize;
    let mut runtime_lagged_starts_nodes = 0usize;
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

    for line in latest_by_node.values() {
        let Some(metrics) = parse_runner_load_metrics(&line.payload) else {
            continue;
        };

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
        if let Some(value) = metrics.runtime_lagged_starts {
            runtime_lagged_starts = runtime_lagged_starts.saturating_add(value);
            runtime_lagged_starts_nodes += 1;
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

    Some(ConsolidatedLoadMetrics {
        total_started: (total_started_nodes > 0).then_some(total_started),
        total_sent,
        total_success,
        total_error,
        http_started: (http_started_nodes > 0).then_some(http_started),
        http_completed: (http_completed_nodes > 0).then_some(http_completed),
        dispatch_submitted: (dispatch_submitted_nodes > 0).then_some(dispatch_submitted),
        http_send_returned: (http_send_returned_nodes > 0).then_some(http_send_returned),
        response_body_completed: (response_body_completed_nodes > 0)
            .then_some(response_body_completed),
        dependency_limited_starts: (dependency_limited_starts_nodes > 0)
            .then_some(dependency_limited_starts),
        runtime_lagged_starts: (runtime_lagged_starts_nodes > 0).then_some(runtime_lagged_starts),
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
        curve_adherence: (scheduled_starts_nodes > 0).then(|| {
            if scheduled_starts == 0 {
                100.0
            } else {
                let value = ((scheduled_starts.saturating_sub(missed_starts)) as f64
                    / scheduled_starts as f64)
                    * 100.0;
                (value * 100.0).round() / 100.0
            }
        }),
        avg_latency: latency.avg_latency,
        p95: latency.p95,
        p99: latency.p99,
        start_time,
        elapsed_ms,
        nodes_reporting,
    })
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
    use std::collections::HashMap;
    use std::sync::Arc;

    use serde_json::json;
    use tokio::sync::Mutex;

    use crate::server::execution::load_batch::{
        add_load_context_fields, build_rps_history_sample, consolidate_load_metrics,
        drain_load_chunk, summarize_load_latency,
    };
    use crate::server::models::{
        ConsolidatedLoadMetrics, LoadEventContext, LoadLatencyAccumulator, LoadLatencySummary,
        NodePlan, RunnerLoadLine,
    };

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
    fn consolidates_dispatch_metrics() {
        let latest = HashMap::from([
            (
                "http://runner-a:3000".to_owned(),
                RunnerLoadLine {
                    node: "http://runner-a:3000".to_owned(),
                    runner_event: "metrics".to_owned(),
                    received_at: 1,
                    payload: json!({
                        "totalSent": 10,
                        "totalSuccess": 8,
                        "totalError": 2,
                        "rps": 100.0,
                        "startTime": 1_000,
                        "elapsedMs": 2_000,
                        "scheduledStarts": 100,
                        "missedStarts": 5,
                        "dispatchSubmitted": 100,
                        "httpSendReturned": 80,
                        "responseBodyCompleted": 70,
                        "dependencyLimitedStarts": 1,
                        "runtimeLaggedStarts": 2,
                        "readyRequests": 20,
                        "activePipelines": 50,
                        "outstandingRequests": 30,
                        "curveAdherence": 95.0
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
                        "totalSent": 20,
                        "totalSuccess": 19,
                        "totalError": 1,
                        "rps": 120.0,
                        "startTime": 900,
                        "elapsedMs": 2_100,
                        "scheduledStarts": 100,
                        "missedStarts": 15,
                        "dispatchSubmitted": 100,
                        "httpSendReturned": 90,
                        "responseBodyCompleted": 85,
                        "dependencyLimitedStarts": 3,
                        "runtimeLaggedStarts": 4,
                        "readyRequests": 30,
                        "activePipelines": 60,
                        "outstandingRequests": 40,
                        "curveAdherence": 85.0
                    }),
                },
            ),
        ]);

        let consolidated = consolidate_load_metrics(&latest, LoadLatencySummary::default())
            .expect("expected consolidated data");

        assert_eq!(consolidated.scheduled_starts, Some(200));
        assert_eq!(consolidated.missed_starts, Some(20));
        assert_eq!(consolidated.dispatch_submitted, Some(200));
        assert_eq!(consolidated.http_send_returned, Some(170));
        assert_eq!(consolidated.response_body_completed, Some(155));
        assert_eq!(consolidated.dependency_limited_starts, Some(4));
        assert_eq!(consolidated.runtime_lagged_starts, Some(6));
        assert_eq!(consolidated.ready_requests, Some(50));
        assert_eq!(consolidated.active_pipelines, Some(110));
        assert_eq!(consolidated.outstanding_requests, Some(70));
        assert_eq!(consolidated.curve_adherence, Some(90.0));
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
            http_send_returned: None,
            response_body_completed: None,
            dependency_limited_starts: None,
            runtime_lagged_starts: None,
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
            elapsed_ms: 2_000,
            nodes_reporting: 2,
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
                    "rps": 21.5,
                    "startTime": 1_000,
                    "elapsedMs": 2_000
                }),
            },
        )]);

        assert_eq!(
            build_rps_history_sample(1_500, &metrics, &latest),
            json!({
                "timestamp": 1_500,
                "rps": 21.5,
                "totalStarted": 45,
                "totalSent": 42,
                "httpStarted": 60,
                "httpCompleted": 58,
                "targetIntensity": 75.0,
                "targetRpsLimit": 150.0,
                "runners": [{
                    "runnerId": "http://runner-a:3000",
                    "httpStarted": 60,
                    "httpCompleted": 58,
                    "totalStarted": 45,
                    "totalSent": 42,
                    "rps": 21.5
                }]
            })
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
}
