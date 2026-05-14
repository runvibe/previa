use std::sync::Arc;

use serde_json::Value;
use tokio::sync::Mutex;

use crate::server::models::{ConsolidatedLoadMetrics, E2eHistoryAccumulator};

pub fn determine_e2e_history_status(cancelled: bool, snapshot: &E2eHistoryAccumulator) -> String {
    if cancelled {
        return "cancelled".to_owned();
    }
    if !snapshot.errors.is_empty() {
        return "error".to_owned();
    }
    if snapshot
        .steps
        .iter()
        .any(|step| step.get("status").and_then(Value::as_str) == Some("error"))
    {
        return "error".to_owned();
    }
    if snapshot
        .summary
        .as_ref()
        .and_then(|summary| summary.get("failed"))
        .and_then(Value::as_u64)
        .unwrap_or(0)
        > 0
    {
        return "error".to_owned();
    }
    "success".to_owned()
}

pub fn determine_load_history_status(
    cancelled: bool,
    consolidated: Option<&ConsolidatedLoadMetrics>,
    no_errors_reported: bool,
) -> String {
    if cancelled {
        return "cancelled".to_owned();
    }
    if !no_errors_reported {
        return "error".to_owned();
    }
    if consolidated.is_some_and(|item| item.total_error > 0) {
        return "error".to_owned();
    }
    if consolidated.is_some_and(load_capacity_is_saturated) {
        return "saturated".to_owned();
    }
    if consolidated.is_some_and(load_capacity_is_under_target) {
        return "under_target".to_owned();
    }
    "success".to_owned()
}

fn load_capacity_is_saturated(metrics: &ConsolidatedLoadMetrics) -> bool {
    metrics.sender_lagged_starts.unwrap_or(0) > 0
        || metrics.dispatcher_lagged_starts.unwrap_or(0) > 0
        || metrics.runtime_lagged_starts.unwrap_or(0) > 0
        || metrics.sender_queue_depth.unwrap_or(0) > 0
}

fn load_capacity_is_under_target(metrics: &ConsolidatedLoadMetrics) -> bool {
    let missed = metrics.missed_starts.unwrap_or(0);
    let poor_curve = metrics.curve_adherence.is_some_and(|value| value < 95.0);
    let below_target = metrics
        .target_rps_limit
        .is_some_and(|target| target > 0.0 && metrics.rps < target * 0.95);

    (missed > 0 || poor_curve) && below_target
}

pub async fn capture_e2e_history_event(
    accumulator: &Arc<Mutex<E2eHistoryAccumulator>>,
    event: &str,
    data: &Value,
) {
    let mut lock = accumulator.lock().await;
    match event {
        "step:result" => {
            let failed_assertions = extract_failed_assertions(data);
            let mut step_snapshot = data.clone();
            if !failed_assertions.is_empty() {
                if let Value::Object(map) = &mut step_snapshot {
                    map.insert(
                        "assertFailures".to_owned(),
                        Value::Array(failed_assertions.clone()),
                    );
                }
            }

            lock.steps.push(step_snapshot);
            if !failed_assertions.is_empty() {
                lock.errors
                    .push(format_assert_failure_message(data, &failed_assertions));
            } else if data.get("status").and_then(Value::as_str) == Some("error") {
                lock.errors.push(extract_error_message(data));
            }
        }
        "pipeline:complete" => {
            lock.summary = Some(data.clone());
        }
        "error" => {
            lock.errors.push(extract_error_message(data));
        }
        _ => {}
    }
}

pub async fn push_load_error(load_errors: &Arc<Mutex<Vec<String>>>, message: String) {
    let mut lock = load_errors.lock().await;
    lock.push(message);
}

pub fn extract_error_message(data: &Value) -> String {
    data.get("message")
        .and_then(Value::as_str)
        .or_else(|| data.get("error").and_then(Value::as_str))
        .map(str::to_owned)
        .unwrap_or_else(|| data.to_string())
}

pub fn extract_failed_assertions(data: &Value) -> Vec<Value> {
    data.get("assertResults")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| item.get("passed").and_then(Value::as_bool) == Some(false))
                .cloned()
                .collect::<Vec<Value>>()
        })
        .unwrap_or_default()
}

pub fn format_assert_failure_message(step_data: &Value, failed_assertions: &[Value]) -> String {
    let step_id = step_data
        .get("stepId")
        .and_then(Value::as_str)
        .unwrap_or("unknown_step");
    let details = failed_assertions
        .iter()
        .filter_map(|item| {
            let assertion = item.get("assertion")?;
            let field = assertion
                .get("field")
                .and_then(Value::as_str)
                .unwrap_or("field");
            let operator = assertion
                .get("operator")
                .and_then(Value::as_str)
                .unwrap_or("operator");
            let expected = assertion
                .get("expected")
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_owned());
            let actual = item
                .get("actual")
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_owned());
            Some(format!(
                "{} {} expected={} actual={}",
                field, operator, expected, actual
            ))
        })
        .collect::<Vec<String>>()
        .join("; ");

    if details.is_empty() {
        format!(
            "step {} failed {} assertion(s)",
            step_id,
            failed_assertions.len()
        )
    } else {
        format!(
            "step {} failed {} assertion(s): {}",
            step_id,
            failed_assertions.len(),
            details
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_consolidated_metrics() -> ConsolidatedLoadMetrics {
        ConsolidatedLoadMetrics {
            total_started: Some(1_000),
            total_sent: 1_000,
            total_success: 1_000,
            total_error: 0,
            http_started: Some(1_000),
            http_completed: Some(1_000),
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
            rps: 100.0,
            target_intensity: None,
            target_rps_limit: Some(100.0),
            in_flight: None,
            runner_max_rps: None,
            tick_ms: None,
            scheduled_starts: Some(1_000),
            missed_starts: None,
            ready_requests: None,
            active_pipelines: None,
            outstanding_requests: None,
            curve_adherence: Some(100.0),
            avg_latency: 10,
            p95: 20,
            p99: 30,
            start_time: 0,
            elapsed_ms: 1_000,
            nodes_reporting: 1,
            lifecycle_buckets: Vec::new(),
        }
    }

    #[test]
    fn load_status_is_under_target_when_curve_adherence_is_low() {
        let mut metrics = test_consolidated_metrics();
        metrics.rps = 400.0;
        metrics.target_rps_limit = Some(2_500.0);
        metrics.curve_adherence = Some(50.0);
        metrics.missed_starts = Some(10_000);

        let status = determine_load_history_status(false, Some(&metrics), true);

        assert_eq!(status, "under_target");
    }

    #[test]
    fn load_status_is_saturated_when_sender_lagged() {
        let mut metrics = test_consolidated_metrics();
        metrics.sender_lagged_starts = Some(1);

        let status = determine_load_history_status(false, Some(&metrics), true);

        assert_eq!(status, "saturated");
    }
}
