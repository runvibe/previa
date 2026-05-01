use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RuntimeSpec {
    pub slug: String,
    #[serde(default)]
    pub servers: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RuntimeEnvGroup {
    pub slug: String,
    #[serde(default)]
    pub urls: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Pipeline {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub description: Option<String>,
    #[schema(min_items = 1)]
    pub steps: Vec<PipelineStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PipelineStep {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    #[schema(value_type = Object, nullable = true)]
    pub body: Option<Value>,
    #[serde(default)]
    #[serde(rename = "operationId")]
    pub operation_id: Option<String>,
    #[serde(default)]
    pub delay: Option<u64>,
    #[serde(default)]
    pub retry: Option<usize>,
    #[serde(default)]
    pub asserts: Vec<StepAssertion>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StepAssertion {
    pub field: String,
    pub operator: String,
    pub expected: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AssertionResult {
    pub assertion: StepAssertion,
    pub passed: bool,
    pub actual: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StepRequest {
    pub method: String,
    pub url: String,
    pub headers: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Object)]
    pub body: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StepResponse {
    pub status: u16,
    #[serde(rename = "statusText")]
    pub status_text: String,
    pub headers: HashMap<String, String>,
    #[schema(value_type = Object)]
    pub body: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StepExecutionResult {
    #[serde(rename = "stepId")]
    pub step_id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<StepRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<StepResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<u128>,
    #[serde(rename = "attempts", skip_serializing_if = "Option::is_none")]
    pub attempts: Option<usize>,
    #[serde(rename = "attempt")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt: Option<usize>,
    #[serde(rename = "maxAttempts")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_attempts: Option<usize>,
    #[serde(rename = "assertResults", skip_serializing_if = "Option::is_none")]
    pub assert_results: Option<Vec<AssertionResult>>,
}
