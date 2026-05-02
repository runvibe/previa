use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use previa_runner::Pipeline;
use previa_runner::RuntimeEnvGroup;
use previa_runner::RuntimeSpec;

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
    pub max_in_flight: usize,
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
pub struct LoadTestMetrics {
    pub total_started: usize,
    pub total_sent: usize,
    pub total_success: usize,
    pub total_error: usize,
    pub http_started: usize,
    pub http_completed: usize,
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
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<RunnerInfoResponse>,
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
