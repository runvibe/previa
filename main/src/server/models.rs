use std::collections::{BTreeMap, HashMap};

use previa_runner::{Pipeline, RuntimeSpec};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoadTestRequest {
    pub pipeline: Pipeline,
    pub config: LoadTestConfig,
    pub selected_base_url_key: Option<String>,
    pub project_id: Option<String>,
    pub pipeline_index: Option<i64>,
    #[serde(default)]
    pub specs: Vec<RuntimeSpec>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct E2eTestRequest {
    pub pipeline: Pipeline,
    pub selected_base_url_key: Option<String>,
    pub project_id: Option<String>,
    pub pipeline_index: Option<i64>,
    #[serde(default)]
    pub specs: Vec<RuntimeSpec>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectE2eTestRequest {
    pub pipeline_id: Option<String>,
    pub pipeline: Option<Pipeline>,
    pub selected_base_url_key: Option<String>,
    pub pipeline_index: Option<i64>,
    #[serde(default)]
    pub specs: Vec<RuntimeSpec>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectE2eQueueRequest {
    pub pipeline_ids: Vec<String>,
    pub selected_base_url_key: Option<String>,
    #[serde(default)]
    pub specs: Vec<RuntimeSpec>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectLoadTestRequest {
    pub pipeline_id: Option<String>,
    pub pipeline: Option<Pipeline>,
    pub config: LoadTestConfig,
    pub selected_base_url_key: Option<String>,
    pub pipeline_index: Option<i64>,
    #[serde(default)]
    pub specs: Vec<RuntimeSpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoadTestConfig {
    pub total_requests: usize,
    pub concurrency: usize,
    pub ramp_up_seconds: f64,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryQuery {
    pub pipeline_index: Option<i64>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub order: Option<HistoryOrder>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectListQuery {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub order: Option<HistoryOrder>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectTransferQuery {
    pub include_history: Option<bool>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectSqliteExportRequest {
    #[serde(default)]
    pub all: bool,
    #[serde(default)]
    pub project_ids: Vec<String>,
    pub include_history: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProxyRequest {
    #[serde(default)]
    pub headers: HashMap<String, String>,
    pub url: String,
    #[schema(value_type = Object, nullable = true)]
    pub body: Option<Value>,
    pub method: String,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectUpsertRequest {
    pub name: String,
    pub description: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    #[schema(value_type = Object, nullable = true)]
    pub spec: Option<Value>,
    #[serde(default)]
    pub pipelines: Vec<PipelineInput>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectMetadataUpsertRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRecord {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PipelineInput {
    pub name: String,
    pub description: Option<String>,
    pub steps: Vec<previa_runner::PipelineStep>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPipelineRecord {
    pub id: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub steps: Vec<previa_runner::PipelineStep>,
    pub runtime: PipelineRuntimeState,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PipelineRuntimeState {
    pub status: PipelineRuntimeStatus,
    pub active_execution: Option<PipelineExecutionRef>,
    pub active_queue: Option<PipelineQueueRef>,
}

impl PipelineRuntimeState {
    pub fn idle() -> Self {
        Self {
            status: PipelineRuntimeStatus::Idle,
            active_execution: None,
            active_queue: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum PipelineRuntimeStatus {
    Idle,
    Queued,
    Running,
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PipelineExecutionKind {
    E2e,
    Load,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PipelineExecutionRef {
    pub id: String,
    pub kind: PipelineExecutionKind,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PipelineQueueRef {
    pub id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpecUrlEntry {
    pub name: String,
    pub url: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectSpecUpsertRequest {
    #[schema(value_type = Object)]
    pub spec: Value,
    pub url: Option<String>,
    pub slug: Option<String>,
    #[serde(default)]
    pub urls: Vec<SpecUrlEntry>,
    #[serde(default)]
    pub servers: HashMap<String, String>,
    #[serde(default)]
    pub sync: bool,
    #[serde(default)]
    pub live: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSpecRecord {
    pub id: String,
    pub project_id: String,
    #[schema(value_type = Object)]
    pub spec: Value,
    pub spec_md5: String,
    pub url: Option<String>,
    pub slug: Option<String>,
    pub urls: Vec<SpecUrlEntry>,
    pub servers: HashMap<String, String>,
    pub sync: bool,
    pub live: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectHistoryExport {
    pub e2e: Vec<E2eHistoryRecord>,
    pub load: Vec<LoadHistoryRecord>,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectExportProject {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[schema(value_type = Object, nullable = true)]
    pub spec: Option<Value>,
    pub pipelines: Vec<Pipeline>,
    pub specs: Vec<ProjectSpecRecord>,
    pub history: ProjectHistoryExport,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectExportEnvelope {
    pub format: String,
    pub exported_at: String,
    pub history_included: bool,
    pub project: ProjectExportProject,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectImportResponse {
    pub project_id: String,
    pub include_history: bool,
    pub pipelines_imported: usize,
    pub specs_imported: usize,
    pub e2e_history_imported: usize,
    pub load_history_imported: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PipelineImportRequest {
    pub stack_name: String,
    #[schema(value_type = Vec<Object>)]
    pub pipelines: Vec<Pipeline>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PipelineImportResponse {
    pub project_id: String,
    pub stack_name: String,
    pub pipelines_imported: usize,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenApiValidationRequest {
    #[schema(value_type = String)]
    pub source: String,
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum OpenApiValidationSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OpenApiValidationStatus {
    Valid,
    Invalid,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OpenApiValidationPoint {
    pub severity: OpenApiValidationSeverity,
    pub line: Option<u32>,
    pub pointer: Option<String>,
    pub comment: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OpenApiValidationResponse {
    #[schema(value_type = Object, nullable = true)]
    pub spec: Option<Value>,
    pub source_md5: String,
    pub status: OpenApiValidationStatus,
    pub points: Vec<OpenApiValidationPoint>,
}

#[derive(Debug, Clone, Copy, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum HistoryOrder {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum E2eQueueStatus {
    Pending,
    Running,
    Failed,
    Completed,
    Cancelled,
}

impl E2eQueueStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Failed => "failed",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Failed | Self::Completed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct E2eQueuePipelineRecord {
    pub id: String,
    pub status: E2eQueueStatus,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct E2eQueueRecord {
    pub id: String,
    pub status: E2eQueueStatus,
    pub pipelines: Vec<E2eQueuePipelineRecord>,
    pub updated_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorResponse {
    pub error: String,
    pub message: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CancelExecutionResponse {
    pub execution_id: String,
    pub cancelled: bool,
    pub already_cancelled: bool,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct HistoryMetadata {
    pub project_id: Option<String>,
    pub pipeline_index: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct E2eHistoryRecord {
    pub id: String,
    pub execution_id: String,
    pub transaction_id: Option<String>,
    pub project_id: Option<String>,
    pub pipeline_index: Option<i64>,
    pub pipeline_id: Option<String>,
    pub pipeline_name: String,
    pub selected_base_url_key: Option<String>,
    pub status: String,
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
    pub duration_ms: i64,
    #[schema(value_type = Object, nullable = true)]
    pub summary: Option<Value>,
    #[schema(value_type = Vec<Object>)]
    pub steps: Vec<Value>,
    pub errors: Vec<String>,
    #[schema(value_type = Object)]
    pub request: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoadHistoryRecord {
    pub id: String,
    pub execution_id: String,
    pub transaction_id: Option<String>,
    pub project_id: Option<String>,
    pub pipeline_index: Option<i64>,
    pub pipeline_id: Option<String>,
    pub pipeline_name: String,
    pub selected_base_url_key: Option<String>,
    pub status: String,
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
    pub duration_ms: i64,
    #[schema(value_type = Object)]
    pub requested_config: Value,
    #[schema(value_type = Object, nullable = true)]
    pub final_consolidated: Option<Value>,
    #[schema(value_type = Vec<Object>)]
    pub final_lines: Vec<Value>,
    pub errors: Vec<String>,
    #[schema(value_type = Object)]
    pub request: Value,
    #[schema(value_type = Object)]
    pub context: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunnerRuntimeInfo {
    pub pid: u32,
    pub memory_bytes: u64,
    pub virtual_memory_bytes: u64,
    pub cpu_usage_percent: f32,
    #[serde(default)]
    pub network_tx_bytes: u64,
    #[serde(default)]
    pub network_rx_bytes: u64,
    #[serde(default)]
    pub network_total_bytes: u64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunnerInfo {
    pub endpoint: String,
    pub active: bool,
    pub runtime: Option<RunnerRuntimeInfo>,
    pub runtime_error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunnerRecord {
    pub id: String,
    pub endpoint: String,
    pub name: Option<String>,
    pub source: String,
    pub enabled: bool,
    pub health_status: String,
    pub last_seen_at: Option<String>,
    pub last_error: Option<String>,
    pub runtime: Option<RunnerRuntimeInfo>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunnerUpsertRequest {
    pub endpoint: String,
    pub name: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunnerUpdateRequest {
    pub name: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OrchestratorInfoResponse {
    pub context: String,
    pub total_runners: usize,
    pub active_runners: usize,
    pub runners: Vec<RunnerInfo>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OrchestratorSseEventData {
    pub requested_nodes: Option<usize>,
    pub nodes_found: Option<usize>,
    pub nodes_used: Option<usize>,
    pub registered_nodes_total: Option<usize>,
    pub active_nodes_total: Option<usize>,
    pub used_nodes_total: Option<usize>,
    pub registered_nodes: Option<Vec<String>>,
    pub active_nodes: Option<Vec<String>>,
    pub used_nodes: Option<Vec<String>>,
    pub runners: Option<Vec<String>>,
    pub warning: Option<String>,
    pub runner_load_plan: Option<Vec<RunnerLoadPlanItem>>,
    pub batch_window_ms: Option<u64>,
    pub execution_id: Option<String>,
    pub step_id: Option<String>,
    pub status: Option<String>,
    pub message: Option<String>,
    pub lines: Option<Vec<RunnerLoadLine>>,
    pub consolidated: Option<ConsolidatedLoadMetrics>,
    #[schema(value_type = Object)]
    pub payload: Option<Value>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunnerLoadPlanItem {
    pub node: String,
    pub total_requests: usize,
    pub concurrency: usize,
    pub desired_total_requests: usize,
    pub above_desired: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunnerLoadLine {
    pub node: String,
    pub runner_event: String,
    pub received_at: u64,
    #[schema(value_type = Object)]
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConsolidatedLoadMetrics {
    pub total_sent: usize,
    pub total_success: usize,
    pub total_error: usize,
    pub rps: f64,
    pub avg_latency: u64,
    pub p95: u64,
    pub p99: u64,
    pub start_time: u64,
    pub elapsed_ms: u64,
    pub nodes_reporting: usize,
}

#[derive(Debug, Clone, Default)]
pub struct LoadLatencyAccumulator {
    pub sample_count: usize,
    pub total_duration_ms: u128,
    pub histogram: BTreeMap<u64, usize>,
}

impl LoadLatencyAccumulator {
    pub fn add_sample(&mut self, duration_ms: u64) {
        self.sample_count = self.sample_count.saturating_add(1);
        self.total_duration_ms = self.total_duration_ms.saturating_add(duration_ms as u128);
        let slot = self.histogram.entry(duration_ms).or_insert(0);
        *slot = slot.saturating_add(1);
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct LoadLatencySummary {
    pub avg_latency: u64,
    pub p95: u64,
    pub p99: u64,
}

#[derive(Debug, Clone)]
pub struct NodePlan {
    pub requested_nodes: usize,
    pub nodes_found: usize,
    pub nodes_used: usize,
    pub warning: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LoadEventContext {
    pub plan: NodePlan,
    pub warning: Option<String>,
    pub registered_nodes: Vec<String>,
    pub active_nodes: Vec<String>,
    pub used_nodes: Vec<String>,
    pub runner_load_plan: Vec<RunnerLoadPlanItem>,
    pub batch_window_ms: u64,
}

#[derive(Debug, Clone)]
pub struct SseMessage {
    pub event: String,
    pub data: Value,
}

#[derive(Debug, Clone, Default)]
pub struct E2eHistoryAccumulator {
    pub summary: Option<Value>,
    pub steps: Vec<Value>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RunnerLoadMetricsPoint {
    pub total_sent: usize,
    pub total_success: usize,
    pub total_error: usize,
    pub rps: f64,
    pub start_time: u64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone)]
pub struct E2eHistoryWrite {
    pub id: String,
    pub execution_id: String,
    pub transaction_id: Option<String>,
    pub metadata: HistoryMetadata,
    pub pipeline_id: Option<String>,
    pub pipeline_name: String,
    pub selected_base_url_key: Option<String>,
    pub status: String,
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
    pub duration_ms: i64,
    pub summary: Option<Value>,
    pub steps: Vec<Value>,
    pub errors: Vec<String>,
    pub request: Value,
}

#[derive(Debug, Clone)]
pub struct LoadHistoryWrite {
    pub id: String,
    pub execution_id: String,
    pub transaction_id: Option<String>,
    pub metadata: HistoryMetadata,
    pub pipeline_id: Option<String>,
    pub pipeline_name: String,
    pub selected_base_url_key: Option<String>,
    pub status: String,
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
    pub duration_ms: i64,
    pub requested_config: Value,
    pub final_consolidated: Option<Value>,
    pub final_lines: Vec<Value>,
    pub errors: Vec<String>,
    pub request: Value,
    pub context: Value,
}
