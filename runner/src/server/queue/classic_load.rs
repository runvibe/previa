use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use serde_json::Value;
use tokio::sync::mpsc;
use tracing::error;

use previa_runner::{
    Pipeline, RuntimeEnvGroup, RuntimeSpec, StepExecutionResult,
    execute_pipeline_with_runtime_request_gate,
};

use crate::server::metrics::{MetricsAccumulator, estimate_results_network_bytes};
use crate::server::models::LoadTestConfig;
use crate::server::runtime::RuntimeSampler;
use crate::server::sse::{SseMessage, send_sse_or_cancel};

pub async fn run_classic_load(
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
                let index = counter.fetch_add(1, Ordering::SeqCst);
                if index >= total_requests {
                    break;
                }
                metrics.lock().await.record_start();
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
                            metrics.lock().await.record_http_start();
                            true
                        })
                    },
                )
                .await;
                let duration_ms = start.elapsed().as_millis() as u64;
                let success = !results.iter().any(|result| result.status == "error");
                let (network_tx_bytes, network_rx_bytes) = estimate_results_network_bytes(&results);
                let status = terminal_http_status(&results, success);
                let runtime = runtime_sampler.lock().await.snapshot();
                let snapshot = {
                    let mut metrics = metrics.lock().await;
                    metrics.update(duration_ms as f64, success);
                    if status.is_some() || !success {
                        metrics.record_status_code(status);
                    }
                    metrics.record_http_completed_count(
                        results
                            .iter()
                            .filter(|result| result.request.is_some())
                            .count(),
                    );
                    metrics.add_network_bytes(network_tx_bytes, network_rx_bytes);
                    metrics.snapshot(Some(duration_ms), runtime)
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
        if let Err(error) = handle.await {
            error!("load worker join error: {error}");
        }
    }
    let complete = {
        let metrics = metrics.lock().await;
        let runtime = runtime_sampler.lock().await.snapshot();
        metrics.snapshot(None, runtime)
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
