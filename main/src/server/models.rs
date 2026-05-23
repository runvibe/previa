use std::collections::{BTreeMap, HashMap};

use previa_runner::{Pipeline, RuntimeEnvGroup, RuntimeSpec, StepExecutionResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use crate::server::auth::permissions::Role;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoadTestRequest {
    pub pipeline: Pipeline,
    #[serde(default)]
    pub config: Option<LoadTestConfig>,
    #[serde(default)]
    pub load: Option<LoadProfile>,
    #[serde(default)]
    pub target_rps: Option<u64>,
    pub selected_base_url_key: Option<String>,
    pub selected_env_group_slug: Option<String>,
    pub project_id: Option<String>,
    pub pipeline_index: Option<i64>,
    #[serde(default)]
    pub specs: Vec<RuntimeSpec>,
    #[serde(default)]
    pub env_groups: Vec<RuntimeEnvGroup>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthClientKind {
    App,
    ApiToken,
}

impl Default for AuthClientKind {
    fn default() -> Self {
        Self::App
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthLoginRequest {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub client_kind: AuthClientKind,
    #[serde(default)]
    pub token_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthTokenKind {
    Jwt,
    ApiToken,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthPrincipalSource {
    Env,
    Database,
    Anonymous,
    ApiToken,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuthUserResponse {
    pub id: String,
    pub username: String,
    pub role: Role,
    pub source: AuthPrincipalSource,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiTokenRecord {
    pub id: String,
    pub name: String,
    pub token_prefix: String,
    pub role: Role,
    pub active: bool,
    pub expires_at: Option<String>,
    pub created_by_username: String,
    pub last_used_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuthLoginResponse {
    pub token_kind: AuthTokenKind,
    pub token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<AuthUserResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record: Option<ApiTokenRecord>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApiTokenCreateRequest {
    pub name: String,
    pub role: Role,
    #[serde(default)]
    pub expires_at: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApiTokenUpdateRequest {
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiTokenCreateResponse {
    pub token: String,
    pub record: ApiTokenRecord,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserRecord {
    pub id: String,
    pub username: String,
    pub role: Role,
    pub active: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserCreateRequest {
    pub username: String,
    pub password: String,
    pub role: Role,
    #[serde(default = "default_user_active")]
    pub active: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserUpdateRequest {
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub role: Option<Role>,
    #[serde(default)]
    pub active: Option<bool>,
}

fn default_user_active() -> bool {
    true
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct E2eTestRequest {
    pub pipeline: Pipeline,
    pub selected_base_url_key: Option<String>,
    pub selected_env_group_slug: Option<String>,
    pub project_id: Option<String>,
    pub pipeline_index: Option<i64>,
    pub start_step_id: Option<String>,
    #[serde(default)]
    pub prior_results: HashMap<String, StepExecutionResult>,
    #[serde(default)]
    pub specs: Vec<RuntimeSpec>,
    #[serde(default)]
    pub env_groups: Vec<RuntimeEnvGroup>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectE2eTestRequest {
    pub pipeline_id: Option<String>,
    pub pipeline: Option<Pipeline>,
    pub selected_base_url_key: Option<String>,
    pub selected_env_group_slug: Option<String>,
    pub pipeline_index: Option<i64>,
    #[serde(default)]
    pub specs: Vec<RuntimeSpec>,
    #[serde(default)]
    pub env_groups: Vec<RuntimeEnvGroup>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectE2eRerunFromStepRequest {
    pub pipeline_id: Option<String>,
    pub pipeline: Option<Pipeline>,
    pub start_step_id: String,
    #[serde(default)]
    pub prior_results: HashMap<String, StepExecutionResult>,
    pub selected_base_url_key: Option<String>,
    pub selected_env_group_slug: Option<String>,
    pub pipeline_index: Option<i64>,
    #[serde(default)]
    pub specs: Vec<RuntimeSpec>,
    #[serde(default)]
    pub env_groups: Vec<RuntimeEnvGroup>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectE2eQueueRequest {
    pub pipeline_ids: Vec<String>,
    pub selected_base_url_key: Option<String>,
    pub selected_env_group_slug: Option<String>,
    #[serde(default)]
    pub specs: Vec<RuntimeSpec>,
    #[serde(default)]
    pub env_groups: Vec<RuntimeEnvGroup>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectLoadTestRequest {
    pub pipeline_id: Option<String>,
    pub pipeline: Option<Pipeline>,
    #[serde(default)]
    pub config: Option<LoadTestConfig>,
    #[serde(default)]
    pub load: Option<LoadProfile>,
    #[serde(default)]
    pub target_rps: Option<u64>,
    pub selected_base_url_key: Option<String>,
    pub selected_env_group_slug: Option<String>,
    pub pipeline_index: Option<i64>,
    #[serde(default)]
    pub specs: Vec<RuntimeSpec>,
    #[serde(default)]
    pub env_groups: Vec<RuntimeEnvGroup>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoadCapacityPreviewRequest {
    pub target_rps: u64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoadCapacityPreviewResponse {
    pub target_rps: u64,
    pub rps_per_runner: u64,
    pub estimated_runner_count: usize,
    pub capacity_mode: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoadExecutionStartResponse {
    pub execution_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct KubernetesReservationCreateRequest {
    pub execution_id: String,
    pub pipeline_id: String,
    pub count: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct KubernetesReservationStatus {
    pub reservation_id: String,
    pub status: String,
    pub requested_runners: usize,
    pub ready_runners: usize,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub first_execution_started_at: Option<String>,
    #[serde(default)]
    pub idle_since: Option<String>,
    #[serde(default)]
    pub reservation_token: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub runners: Vec<KubernetesReservationRunner>,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct KubernetesReservationRunner {
    pub id: String,
    pub endpoint: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunnerReservationRecord {
    pub execution_id: String,
    pub pipeline_id: Option<String>,
    pub capacity_mode: String,
    pub requested_runner_count: usize,
    pub ready_runner_count: usize,
    pub target_rps: u64,
    pub node_profile: Option<String>,
    pub reservation_id: Option<String>,
    #[serde(skip_serializing)]
    #[schema(ignore)]
    pub reservation_token: Option<String>,
    pub reservation_expires_at: Option<String>,
    pub reservation_status: String,
    pub runner_endpoints: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct RunnerReservationUpsert {
    pub execution_id: String,
    pub pipeline_id: Option<String>,
    pub capacity_mode: String,
    pub requested_runner_count: usize,
    pub ready_runner_count: usize,
    pub target_rps: u64,
    pub node_profile: Option<String>,
    pub reservation_id: Option<String>,
    pub reservation_token: Option<String>,
    pub reservation_expires_at: Option<String>,
    pub reservation_status: String,
    pub runner_endpoints: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
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
    #[serde(default)]
    pub runner_max_rps: Option<f64>,
    #[serde(default)]
    pub grace_period_ms: Option<u64>,
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
    #[serde(default)]
    pub tags: Vec<String>,
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
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRecord {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub tags: Vec<String>,
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

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvGroupEntry {
    pub name: String,
    pub url: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectEnvGroupUpsertRequest {
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub entries: Vec<EnvGroupEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectEnvGroupRecord {
    pub id: String,
    pub project_id: String,
    pub slug: String,
    pub name: String,
    pub entries: Vec<EnvGroupEntry>,
    pub created_at: String,
    pub updated_at: String,
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
    #[serde(default)]
    pub tags: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    #[schema(value_type = Object, nullable = true)]
    pub spec: Option<Value>,
    pub pipelines: Vec<Pipeline>,
    pub specs: Vec<ProjectSpecRecord>,
    pub env_groups: Vec<ProjectEnvGroupRecord>,
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
    pub env_groups_imported: usize,
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
    #[serde(default)]
    pub busy: bool,
    #[serde(default)]
    pub started_execution_count: u64,
    #[serde(default)]
    pub last_started_at: Option<String>,
    #[serde(default)]
    pub last_finished_at: Option<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerLoadSnapshotMode {
    Live,
    Final,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConsolidatedLoadMetrics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_started: Option<usize>,
    pub total_sent: usize,
    pub total_success: usize,
    pub total_error: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_started: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_completed: Option<usize>,
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
    pub rps: f64,
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
    pub avg_latency: u64,
    pub p95: u64,
    pub p99: u64,
    pub start_time: u64,
    pub elapsed_ms: u64,
    pub nodes_reporting: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lifecycle_buckets: Vec<ConsolidatedLoadLifecycleBucket>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConsolidatedLoadLifecycleBucket {
    pub elapsed_ms: u64,
    pub planned: usize,
    pub slot_enqueued: usize,
    pub request_prepared: usize,
    pub request_enqueued: usize,
    pub send_task_spawned: usize,
    pub send_started: usize,
    pub http_started: usize,
    pub http_send_returned: usize,
    pub response_body_completed: usize,
    pub dispatcher_lagged: usize,
    pub runtime_lagged: usize,
    pub sender_lagged: usize,
    pub sender_start_lag_ms_max: u64,
    pub http_send_duration_ms_max: u64,
    pub response_observation_duration_ms_max: u64,
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
pub struct RunnerLoadLatencyBucket {
    pub duration_ms: u64,
    pub count: usize,
}

#[derive(Debug, Clone)]
pub struct RunnerLoadDispatchBucket {
    pub elapsed_ms: u64,
    pub count: usize,
}

#[derive(Debug, Clone, Default)]
pub struct RunnerLoadLifecycleBucket {
    pub elapsed_ms: u64,
    pub planned: usize,
    pub slot_enqueued: usize,
    pub request_prepared: usize,
    pub request_enqueued: usize,
    pub send_task_spawned: usize,
    pub send_started: usize,
    pub http_started: usize,
    pub http_send_returned: usize,
    pub response_body_completed: usize,
    pub dispatcher_lagged: usize,
    pub runtime_lagged: usize,
    pub sender_lagged: usize,
    pub sender_start_lag_ms_max: u64,
    pub http_send_duration_ms_max: u64,
    pub response_observation_duration_ms_max: u64,
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
    pub snapshot_mode: Option<RunnerLoadSnapshotMode>,
    pub total_started: Option<usize>,
    pub total_sent: usize,
    pub total_success: usize,
    pub total_error: usize,
    pub http_started: Option<usize>,
    pub http_completed: Option<usize>,
    pub dispatch_submitted: Option<usize>,
    pub dispatch_started: Option<usize>,
    pub http_send_returned: Option<usize>,
    pub response_body_completed: Option<usize>,
    pub dependency_limited_starts: Option<usize>,
    pub dispatcher_lagged_starts: Option<usize>,
    pub runtime_lagged_starts: Option<usize>,
    pub sender_lagged_starts: Option<usize>,
    pub sender_queue_depth: Option<usize>,
    pub sender_start_lag_avg_ms: Option<f64>,
    pub sender_start_lag_p95_ms: Option<u64>,
    pub sender_start_lag_p99_ms: Option<u64>,
    pub sender_start_lag_max_ms: Option<u64>,
    pub http_send_duration_avg_ms: Option<f64>,
    pub http_send_duration_p95_ms: Option<u64>,
    pub http_send_duration_p99_ms: Option<u64>,
    pub response_observation_duration_avg_ms: Option<f64>,
    pub response_observation_duration_p95_ms: Option<u64>,
    pub response_observation_duration_p99_ms: Option<u64>,
    pub scheduler_lag_ms: Option<u64>,
    pub scheduler_lagged_starts: Option<usize>,
    pub slot_enqueued: Option<usize>,
    pub request_prepared: Option<usize>,
    pub request_enqueued: Option<usize>,
    pub send_task_spawned: Option<usize>,
    pub send_started: Option<usize>,
    pub rps: f64,
    pub start_time: u64,
    pub elapsed_ms: u64,
    pub target_intensity: Option<f64>,
    pub target_rps_limit: Option<f64>,
    pub in_flight: Option<usize>,
    pub runner_max_rps: Option<f64>,
    pub tick_ms: Option<u64>,
    pub scheduled_starts: Option<usize>,
    pub missed_starts: Option<usize>,
    pub ready_requests: Option<usize>,
    pub active_pipelines: Option<usize>,
    pub outstanding_requests: Option<usize>,
    pub curve_adherence: Option<f64>,
    pub latency_sample_count: Option<usize>,
    pub latency_total_duration_ms: Option<u64>,
    pub latency_buckets: Vec<RunnerLoadLatencyBucket>,
    pub dispatch_buckets: Vec<RunnerLoadDispatchBucket>,
    pub lifecycle_buckets: Vec<RunnerLoadLifecycleBucket>,
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
