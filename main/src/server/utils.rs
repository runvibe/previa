use chrono::{SecondsFormat, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::server::models::{
    RunnerLoadDispatchBucket, RunnerLoadLatencyBucket, RunnerLoadLifecycleBucket,
    RunnerLoadMetricsPoint, RunnerLoadSnapshotMode,
};

pub fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

pub fn new_uuid_v7() -> String {
    Uuid::now_v7().to_string()
}

pub fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn parse_runner_load_metrics(payload: &Value) -> Option<RunnerLoadMetricsPoint> {
    let total_sent = get_usize_field(payload, "totalSent")?;
    let total_success = get_usize_field(payload, "totalSuccess")?;
    let total_error = get_usize_field(payload, "totalError")?;
    let rps = get_f64_field(payload, "rps")?;
    let start_time = get_u64_field(payload, "startTime")?;
    let elapsed_ms = get_u64_field(payload, "elapsedMs")?;

    Some(RunnerLoadMetricsPoint {
        snapshot_mode: parse_runner_load_snapshot_mode(payload),
        total_started: get_usize_field(payload, "totalStarted"),
        total_sent,
        total_success,
        total_error,
        http_started: get_usize_field(payload, "httpStarted"),
        http_completed: get_usize_field(payload, "httpCompleted"),
        dispatch_submitted: get_usize_field(payload, "dispatchSubmitted"),
        dispatch_started: get_usize_field(payload, "dispatchStarted"),
        http_send_returned: get_usize_field(payload, "httpSendReturned"),
        response_body_completed: get_usize_field(payload, "responseBodyCompleted"),
        dependency_limited_starts: get_usize_field(payload, "dependencyLimitedStarts"),
        dispatcher_lagged_starts: get_usize_field(payload, "dispatcherLaggedStarts"),
        runtime_lagged_starts: get_usize_field(payload, "runtimeLaggedStarts"),
        sender_lagged_starts: get_usize_field(payload, "senderLaggedStarts"),
        sender_queue_depth: get_usize_field(payload, "senderQueueDepth"),
        sender_start_lag_avg_ms: get_f64_field(payload, "senderStartLagAvgMs"),
        sender_start_lag_p95_ms: get_u64_field(payload, "senderStartLagP95Ms"),
        sender_start_lag_p99_ms: get_u64_field(payload, "senderStartLagP99Ms"),
        sender_start_lag_max_ms: get_u64_field(payload, "senderStartLagMaxMs"),
        http_send_duration_avg_ms: get_f64_field(payload, "httpSendDurationAvgMs"),
        http_send_duration_p95_ms: get_u64_field(payload, "httpSendDurationP95Ms"),
        http_send_duration_p99_ms: get_u64_field(payload, "httpSendDurationP99Ms"),
        response_observation_duration_avg_ms: get_f64_field(
            payload,
            "responseObservationDurationAvgMs",
        ),
        response_observation_duration_p95_ms: get_u64_field(
            payload,
            "responseObservationDurationP95Ms",
        ),
        response_observation_duration_p99_ms: get_u64_field(
            payload,
            "responseObservationDurationP99Ms",
        ),
        scheduler_lag_ms: get_u64_field(payload, "schedulerLagMs"),
        scheduler_lagged_starts: get_usize_field(payload, "schedulerLaggedStarts"),
        slot_enqueued: get_usize_field(payload, "slotEnqueued"),
        request_prepared: get_usize_field(payload, "requestPrepared"),
        request_enqueued: get_usize_field(payload, "requestEnqueued"),
        send_task_spawned: get_usize_field(payload, "sendTaskSpawned"),
        send_started: get_usize_field(payload, "sendStarted"),
        rps,
        start_time,
        elapsed_ms,
        target_intensity: get_f64_field(payload, "targetIntensity"),
        target_rps_limit: get_f64_field(payload, "targetRpsLimit"),
        in_flight: get_usize_field(payload, "inFlight"),
        runner_max_rps: get_f64_field(payload, "runnerMaxRps"),
        tick_ms: get_u64_field(payload, "tickMs"),
        scheduled_starts: get_usize_field(payload, "scheduledStarts"),
        missed_starts: get_usize_field(payload, "missedStarts"),
        ready_requests: get_usize_field(payload, "readyRequests"),
        active_pipelines: get_usize_field(payload, "activePipelines"),
        outstanding_requests: get_usize_field(payload, "outstandingRequests"),
        curve_adherence: get_f64_field(payload, "curveAdherence"),
        latency_sample_count: get_usize_field(payload, "latencySampleCount"),
        latency_total_duration_ms: get_u64_field(payload, "latencyTotalDurationMs"),
        latency_buckets: parse_latency_buckets(payload),
        dispatch_buckets: parse_dispatch_buckets(payload),
        lifecycle_buckets: parse_lifecycle_buckets(payload),
    })
}

fn parse_runner_load_snapshot_mode(payload: &Value) -> Option<RunnerLoadSnapshotMode> {
    match payload.get("snapshotMode").and_then(Value::as_str) {
        Some("live") => Some(RunnerLoadSnapshotMode::Live),
        Some("final") => Some(RunnerLoadSnapshotMode::Final),
        _ => None,
    }
}

pub fn parse_runner_duration_ms(payload: &Value) -> Option<u64> {
    get_u64_field(payload, "durationMs")
}

pub fn get_usize_field(payload: &Value, key: &str) -> Option<usize> {
    payload
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

pub fn get_u64_field(payload: &Value, key: &str) -> Option<u64> {
    payload.get(key).and_then(Value::as_u64)
}

pub fn get_f64_field(payload: &Value, key: &str) -> Option<f64> {
    payload.get(key).and_then(Value::as_f64)
}

fn parse_latency_buckets(payload: &Value) -> Vec<RunnerLoadLatencyBucket> {
    payload
        .get("latencyBuckets")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let duration_ms = get_u64_field(item, "durationMs")?;
                    let count = get_usize_field(item, "count")?;
                    Some(RunnerLoadLatencyBucket { duration_ms, count })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_dispatch_buckets(payload: &Value) -> Vec<RunnerLoadDispatchBucket> {
    payload
        .get("dispatchBuckets")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let elapsed_ms = get_u64_field(item, "elapsedMs")?;
                    let count = get_usize_field(item, "count")?;
                    Some(RunnerLoadDispatchBucket { elapsed_ms, count })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_lifecycle_buckets(payload: &Value) -> Vec<RunnerLoadLifecycleBucket> {
    payload
        .get("lifecycleBuckets")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    Some(RunnerLoadLifecycleBucket {
                        elapsed_ms: get_u64_field(item, "elapsedMs")?,
                        planned: get_usize_field(item, "planned").unwrap_or(0),
                        slot_enqueued: get_usize_field(item, "slotEnqueued").unwrap_or(0),
                        request_prepared: get_usize_field(item, "requestPrepared").unwrap_or(0),
                        request_enqueued: get_usize_field(item, "requestEnqueued").unwrap_or(0),
                        send_task_spawned: get_usize_field(item, "sendTaskSpawned").unwrap_or(0),
                        send_started: get_usize_field(item, "sendStarted").unwrap_or(0),
                        http_started: get_usize_field(item, "httpStarted").unwrap_or(0),
                        http_send_returned: get_usize_field(item, "httpSendReturned").unwrap_or(0),
                        response_body_completed: get_usize_field(item, "responseBodyCompleted")
                            .unwrap_or(0),
                        dispatcher_lagged: get_usize_field(item, "dispatcherLagged").unwrap_or(0),
                        runtime_lagged: get_usize_field(item, "runtimeLagged").unwrap_or(0),
                        sender_lagged: get_usize_field(item, "senderLagged").unwrap_or(0),
                        sender_start_lag_ms_max: get_u64_field(item, "senderStartLagMsMax")
                            .unwrap_or(0),
                        http_send_duration_ms_max: get_u64_field(item, "httpSendDurationMsMax")
                            .unwrap_or(0),
                        response_observation_duration_ms_max: get_u64_field(
                            item,
                            "responseObservationDurationMsMax",
                        )
                        .unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}
