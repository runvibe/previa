use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use previa_runner::{
    Pipeline, RuntimeEnvGroup, RuntimeSpec, StepExecutionResult,
    execute_pipeline_from_step_with_client_runtime_hooks, execute_pipeline_with_runtime_hooks,
};

use super::repository::ClaimedJob;
use super::worker::{EventSink, JobExecutor, JobOutcome};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct E2eJobPayload {
    pub pipeline: Pipeline,
    pub selected_base_url_key: Option<String>,
    pub selected_env_group_slug: Option<String>,
    #[serde(default)]
    pub specs: Vec<RuntimeSpec>,
    #[serde(default)]
    pub env_groups: Vec<RuntimeEnvGroup>,
    pub start_step_id: Option<String>,
    #[serde(default)]
    pub prior_results: HashMap<String, StepExecutionResult>,
    pub transaction_id: Option<String>,
}

#[derive(Default)]
pub struct E2eQueueExecutor;

#[async_trait]
impl JobExecutor for E2eQueueExecutor {
    async fn execute(
        &self,
        job: ClaimedJob,
        events: EventSink,
        cancel: CancellationToken,
    ) -> JobOutcome {
        let payload: E2eJobPayload = match serde_json::from_value(job.payload_json) {
            Ok(payload) => payload,
            Err(error) => {
                return JobOutcome::Failed {
                    error: format!("invalid E2E job payload: {error}"),
                    result: json!({"error": "invalid_payload"}),
                    retryable: false,
                };
            }
        };
        if payload.pipeline.steps.is_empty() {
            return JobOutcome::Failed {
                error: "pipeline must contain at least one step".to_owned(),
                result: json!({"error": "empty_pipeline"}),
                retryable: false,
            };
        }

        let _ = events
            .push(
                "execution:running",
                0,
                json!({
                    "executionId": job.execution_id,
                    "jobId": job.job_id,
                    "attempt": job.attempt
                }),
            )
            .await;
        let started_at = std::time::Instant::now();

        let results = if let Some(start_step_id) = payload.start_step_id.as_deref() {
            let client = reqwest::Client::new();
            let start_events = events.clone();
            let result_events = events.clone();
            let cancel_check = cancel.clone();
            execute_pipeline_from_step_with_client_runtime_hooks(
                &client,
                &payload.pipeline,
                start_step_id,
                payload.prior_results,
                Some(payload.specs.as_slice()),
                Some(payload.env_groups.as_slice()),
                payload.selected_env_group_slug.as_deref(),
                move |step_id| {
                    let _ = start_events.try_push(
                        "step:start",
                        started_at.elapsed().as_millis() as i64,
                        json!({"stepId": step_id}),
                    );
                },
                move |result| {
                    let _ = result_events.try_push(
                        "step:result",
                        started_at.elapsed().as_millis() as i64,
                        serde_json::to_value(result).unwrap_or(Value::Null),
                    );
                },
                move || cancel_check.is_cancelled(),
                |_| Box::pin(async { true }),
            )
            .await
        } else {
            let start_events = events.clone();
            let result_events = events.clone();
            let cancel_check = cancel.clone();
            execute_pipeline_with_runtime_hooks(
                &payload.pipeline,
                payload.selected_base_url_key.as_deref(),
                Some(payload.specs.as_slice()),
                Some(payload.env_groups.as_slice()),
                payload.selected_env_group_slug.as_deref(),
                move |step_id| {
                    let _ = start_events.try_push(
                        "step:start",
                        started_at.elapsed().as_millis() as i64,
                        json!({"stepId": step_id}),
                    );
                },
                move |result| {
                    let _ = result_events.try_push(
                        "step:result",
                        started_at.elapsed().as_millis() as i64,
                        serde_json::to_value(result).unwrap_or(Value::Null),
                    );
                },
                move || cancel_check.is_cancelled(),
            )
            .await
        };

        if cancel.is_cancelled() {
            return JobOutcome::Cancelled(json!({"results": results}));
        }
        let passed = results
            .iter()
            .filter(|result| result.status == "success")
            .count();
        let failed = results
            .iter()
            .filter(|result| result.status == "error")
            .count();
        let result_json = json!({
            "results": results,
            "summary": {
                "totalSteps": passed + failed,
                "passed": passed,
                "failed": failed,
                "totalDuration": results.iter()
                    .map(|result| result.duration.unwrap_or(0))
                    .sum::<u128>()
            }
        });
        let _ = events
            .push(
                "pipeline:complete",
                started_at.elapsed().as_millis() as i64,
                result_json.clone(),
            )
            .await;
        if failed > 0 {
            JobOutcome::Failed {
                error: format!("{failed} pipeline step(s) failed"),
                result: result_json,
                retryable: false,
            }
        } else {
            JobOutcome::Completed(result_json)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_incomplete_payload_before_execution() {
        let error = serde_json::from_value::<E2eJobPayload>(json!({}))
            .expect_err("immutable payload must be complete");
        assert!(error.to_string().contains("pipeline"));
    }
}
