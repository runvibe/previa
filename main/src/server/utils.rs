use chrono::{SecondsFormat, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::server::models::RunnerLoadMetricsPoint;

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
        total_started: get_usize_field(payload, "totalStarted"),
        total_sent,
        total_success,
        total_error,
        http_started: get_usize_field(payload, "httpStarted"),
        http_completed: get_usize_field(payload, "httpCompleted"),
        rps,
        start_time,
        elapsed_ms,
        target_intensity: get_f64_field(payload, "targetIntensity"),
        target_rps_limit: get_f64_field(payload, "targetRpsLimit"),
        in_flight: get_usize_field(payload, "inFlight"),
        runner_max_rps: get_f64_field(payload, "runnerMaxRps"),
        tick_ms: get_u64_field(payload, "tickMs"),
    })
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
