use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use previa_runner::Pipeline;
use previa_runner::RuntimeEnvGroup;
use previa_runner::RuntimeSpec;
use previa_runner::StepExecutionResult;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct E2eTestRequest {
    pub pipeline: Pipeline,
    pub selected_base_url_key: Option<String>,
    pub selected_env_group_slug: Option<String>,
    #[serde(default)]
    pub specs: Vec<RuntimeSpec>,
    #[serde(default)]
    pub env_groups: Vec<RuntimeEnvGroup>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct E2eRerunFromStepRequest {
    pub pipeline: Pipeline,
    pub start_step_id: String,
    #[serde(default)]
    pub prior_results: std::collections::HashMap<String, StepExecutionResult>,
    pub selected_env_group_slug: Option<String>,
    #[serde(default)]
    pub specs: Vec<RuntimeSpec>,
    #[serde(default)]
    pub env_groups: Vec<RuntimeEnvGroup>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoadTestRequest {
    pub pipeline: Pipeline,
    #[serde(default)]
    pub config: Option<LoadTestConfig>,
    #[serde(default)]
    pub load: Option<LoadProfile>,
    pub selected_base_url_key: Option<String>,
    pub selected_env_group_slug: Option<String>,
    #[serde(default)]
    pub specs: Vec<RuntimeSpec>,
    #[serde(default)]
    pub env_groups: Vec<RuntimeEnvGroup>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoadTestConfig {
    pub total_requests: usize,
    pub concurrency: usize,
    pub ramp_up_seconds: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LoadProfile {
    pub points: Vec<LoadPoint>,
    #[serde(default)]
    pub interpolation: LoadInterpolation,
    pub runner_max_rps: f64,
    #[serde(default = "default_load_grace_period_ms")]
    pub grace_period_ms: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LoadPoint {
    pub at_ms: u64,
    pub intensity: f64,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LoadInterpolation {
    Smooth,
    Linear,
    Step,
}

impl Default for LoadInterpolation {
    fn default() -> Self {
        Self::Smooth
    }
}

fn default_load_grace_period_ms() -> u64 {
    30_000
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct E2eSummary {
    pub total_steps: usize,
    pub passed: usize,
    pub failed: usize,
    pub total_duration: u128,
}

#[derive(Debug, Serialize, Clone, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoadLatencyBucket {
    pub duration_ms: u64,
    pub count: usize,
}

#[derive(Debug, Serialize, Clone, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoadDispatchBucket {
    pub elapsed_ms: u64,
    pub count: usize,
}

#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum LoadMetricsSnapshotMode {
    Live,
    Final,
}

#[derive(Debug, Serialize, Clone, Default, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoadLifecycleBucket {
    pub elapsed_ms: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub planned: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub slot_enqueued: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub request_prepared: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub request_enqueued: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub send_task_spawned: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub send_started: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub http_started: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub http_send_returned: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub response_body_completed: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub dispatcher_lagged: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub runtime_lagged: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub sender_lagged: usize,
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub sender_start_lag_ms_max: u64,
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub http_send_duration_ms_max: u64,
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub response_observation_duration_ms_max: u64,
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

#[derive(Debug, Serialize, Clone, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoadErrorSample {
    pub step_id: String,
    pub http_status: Option<u16>,
    pub error: String,
    pub count: usize,
}

#[derive(Debug, Serialize, Clone, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoadTestMetrics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_mode: Option<LoadMetricsSnapshotMode>,
    pub total_started: usize,
    pub total_sent: usize,
    pub total_success: usize,
    pub total_error: usize,
    pub http_started: usize,
    pub http_completed: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dispatch_submitted: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dispatch_started: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_send_returned: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_body_completed: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependency_limited_starts: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dispatcher_lagged_starts: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_lagged_starts: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduler_lag_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduler_lagged_starts: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot_enqueued: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_prepared: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_enqueued: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send_task_spawned: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send_started: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_lagged_starts: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_queue_depth: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_start_lag_avg_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_start_lag_p95_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_start_lag_p99_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_start_lag_max_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_send_duration_avg_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_send_duration_p95_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_send_duration_p99_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_observation_duration_avg_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_observation_duration_p95_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_observation_duration_p99_ms: Option<u64>,
    pub rps: f64,
    pub start_time: u64,
    pub elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_intensity: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_rps_limit: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_flight: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runner_max_rps: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tick_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduled_starts: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missed_starts: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ready_requests: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_pipelines: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outstanding_requests: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub curve_adherence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub latency_buckets: Vec<LoadLatencyBucket>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dispatch_buckets: Vec<LoadDispatchBucket>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lifecycle_buckets: Vec<LoadLifecycleBucket>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_sample_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_total_duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub error_samples: Vec<LoadErrorSample>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<RunnerInfoResponse>,
}

impl Default for LoadTestMetrics {
    fn default() -> Self {
        Self {
            snapshot_mode: None,
            total_started: 0,
            total_sent: 0,
            total_success: 0,
            total_error: 0,
            http_started: 0,
            http_completed: 0,
            dispatch_submitted: None,
            dispatch_started: None,
            http_send_returned: None,
            response_body_completed: None,
            dependency_limited_starts: None,
            dispatcher_lagged_starts: None,
            runtime_lagged_starts: None,
            scheduler_lag_ms: None,
            scheduler_lagged_starts: None,
            slot_enqueued: None,
            request_prepared: None,
            request_enqueued: None,
            send_task_spawned: None,
            send_started: None,
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
            rps: 0.0,
            start_time: crate::server::utils::now_ms(),
            elapsed_ms: 0,
            target_intensity: None,
            target_rps_limit: None,
            in_flight: None,
            runner_max_rps: None,
            tick_ms: None,
            scheduled_starts: None,
            missed_starts: None,
            ready_requests: None,
            active_pipelines: None,
            outstanding_requests: None,
            curve_adherence: None,
            duration_ms: None,
            latency_buckets: Vec::new(),
            dispatch_buckets: Vec::new(),
            lifecycle_buckets: Vec::new(),
            latency_sample_count: None,
            latency_total_duration_ms: None,
            error_samples: Vec::new(),
            runtime: None,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorResponse {
    pub error: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunnerInfoResponse {
    pub pid: u32,
    pub memory_bytes: u64,
    pub virtual_memory_bytes: u64,
    pub cpu_usage_percent: f32,
    pub network_tx_bytes: u64,
    pub network_rx_bytes: u64,
    pub network_total_bytes: u64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionInitEvent {
    pub execution_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct StepStartEvent {
    pub step_id: String,
}
