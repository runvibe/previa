use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ReservationStatusKind {
    Provisioning,
    Ready,
    Running,
    Idle,
    Draining,
    Terminating,
    Failed,
    Cancelled,
    Expired,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ReservationFailureReason {
    InsufficientCapacity,
    ProvisionTimeout,
    KubernetesError,
    RunnerHealthTimeout,
    Cancelled,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RunnerLifecycleState {
    Provisioning,
    Reserved,
    Ready,
    Running,
    Idle,
    Draining,
    Terminating,
    Failed,
}

impl RunnerLifecycleState {
    pub fn as_label_value(&self) -> &'static str {
        match self {
            Self::Provisioning => "provisioning",
            Self::Reserved => "reserved",
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Idle => "idle",
            Self::Draining => "draining",
            Self::Terminating => "terminating",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReservationCreateRequest {
    pub execution_id: String,
    pub pipeline_id: String,
    pub count: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReservationStatus {
    pub reservation_id: String,
    pub status: ReservationStatusKind,
    pub requested_runners: usize,
    pub ready_runners: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reservation_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<ReservationFailureReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_execution_started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_since: Option<String>,
    #[serde(default)]
    pub runners: Vec<ReservationRunner>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReservationRunner {
    pub id: String,
    pub endpoint: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct KarpenterProvisionerConfig {
    pub kind: String,
    pub provider: String,
    pub resource_mode: KarpenterResourceMode,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub enum KarpenterResourceMode {
    Managed,
    Reference,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct AwsNodeProfile {
    pub node_pool: String,
    pub ec2_node_class: String,
    pub instance_families: Vec<String>,
    pub instance_sizes: Vec<String>,
    pub expire_after: String,
    pub consolidate_after: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorResponse {
    pub error: String,
    pub message: String,
}
