use std::sync::Arc;

use serde_json::{Value, json};
use sqlx::Row;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::server::execution::{
    resolve_runtime_env_groups_for_execution, resolve_runtime_specs_for_execution,
    send_sse_best_effort, spawn_broadcast_bridge,
};
use crate::server::models::{E2eTestRequest, SseMessage};
use crate::server::queue::config::MainQueueConfig;
use crate::server::queue::repository::{EnqueueExecution, EnqueueJob, QueueRepository};
use crate::server::services::e2e_rerun::validate_rerun_context;
use crate::server::state::{AppState, EXECUTION_SSE_BUFFER_SIZE, ExecutionCtx, ExecutionKind};
use crate::server::validation::pipelines::validate_pipeline_templates;

#[derive(Debug)]
pub enum StartE2eExecutionError {
    BadRequest(String),
    ServiceUnavailable(String),
    Internal(String),
}

#[derive(Debug)]
pub struct E2eExecutionOutcome {
    pub execution_id: String,
    pub status: String,
}

pub struct StartedE2eExecution {
    pub execution_id: String,
    pub subscriber: broadcast::Receiver<SseMessage>,
    pub completion: oneshot::Receiver<E2eExecutionOutcome>,
}

pub async fn start_e2e_execution(
    state: AppState,
    payload: E2eTestRequest,
    transaction_id: Option<String>,
) -> Result<StartedE2eExecution, StartE2eExecutionError> {
    if payload.pipeline.steps.is_empty() {
        return Err(StartE2eExecutionError::BadRequest(
            "pipeline must contain at least one step".to_owned(),
        ));
    }
    if let Some(start_step_id) = payload.start_step_id.as_deref() {
        validate_rerun_context(&payload.pipeline, start_step_id, &payload.prior_results)
            .map_err(StartE2eExecutionError::BadRequest)?;
    }
    let Some(project_id) = payload.project_id.clone() else {
        return Err(StartE2eExecutionError::BadRequest(
            "projectId is required".to_owned(),
        ));
    };

    let runtime_specs =
        resolve_runtime_specs_for_execution(&state.db, Some(&project_id), &payload.specs)
            .await
            .map_err(|error| {
                StartE2eExecutionError::Internal(format!(
                    "failed to load project specs for execution: {error}"
                ))
            })?
            .unwrap_or_default();
    let runtime_env_groups =
        resolve_runtime_env_groups_for_execution(&state.db, Some(&project_id), &payload.env_groups)
            .await
            .map_err(|error| {
                StartE2eExecutionError::Internal(format!(
                    "failed to load project env groups for execution: {error}"
                ))
            })?
            .unwrap_or_default();
    let template_errors = validate_pipeline_templates(
        &payload.pipeline,
        Some(runtime_specs.as_slice()),
        Some(runtime_env_groups.as_slice()),
        payload.selected_env_group_slug.as_deref(),
    );
    if !template_errors.is_empty() {
        return Err(StartE2eExecutionError::BadRequest(
            template_errors.join("; "),
        ));
    }

    let config = MainQueueConfig::from_env().map_err(StartE2eExecutionError::ServiceUnavailable)?;
    let queue = QueueRepository::connect(&config.database_url, 5)
        .await
        .map_err(|error| {
            StartE2eExecutionError::ServiceUnavailable(format!(
                "failed to connect to Postgres execution queue: {error}"
            ))
        })?;
    let protocol = queue
        .protocol_version()
        .await
        .map_err(|error| StartE2eExecutionError::Internal(error.to_string()))?;
    if protocol != previa_runner::queue::QUEUE_PROTOCOL_VERSION.0 {
        return Err(StartE2eExecutionError::ServiceUnavailable(format!(
            "queue protocol mismatch: expected {}, found {protocol}",
            previa_runner::queue::QUEUE_PROTOCOL_VERSION.0
        )));
    }

    let execution_id = Uuid::now_v7();
    let job_id = Uuid::now_v7();
    let execution_id_text = execution_id.to_string();
    let pipeline_id = payload.pipeline.id.clone();
    let job_payload = json!({
        "pipeline": payload.pipeline,
        "selectedBaseUrlKey": payload.selected_base_url_key,
        "selectedEnvGroupSlug": payload.selected_env_group_slug,
        "startStepId": payload.start_step_id,
        "priorResults": payload.prior_results,
        "specs": runtime_specs,
        "envGroups": runtime_env_groups,
        "transactionId": transaction_id,
    });
    queue
        .enqueue_execution(&EnqueueExecution {
            id: execution_id,
            project_id: project_id.clone(),
            pipeline_id: None,
            kind: "e2e".to_owned(),
            request_json: job_payload.clone(),
            created_by: "api".to_owned(),
            transaction_id,
            max_attempts: i32::try_from(config.job_max_attempts).unwrap_or(3),
            jobs: vec![EnqueueJob {
                id: job_id,
                shard_index: None,
                pool: std::env::var("PREVIA_QUEUE_DEFAULT_POOL")
                    .unwrap_or_else(|_| "default".to_owned()),
                requirements_json: json!({}),
                payload_json: job_payload,
                priority: 0,
            }],
        })
        .await
        .map_err(|error| {
            StartE2eExecutionError::Internal(format!("failed to enqueue E2E execution: {error}"))
        })?;

    let init_payload = json!({
        "executionId": execution_id_text,
        "jobId": job_id,
        "status": "queued",
        "transport": "postgres"
    });
    let (sse_tx, _) = broadcast::channel(EXECUTION_SSE_BUFFER_SIZE);
    let subscriber = sse_tx.subscribe();
    let context = Arc::new(ExecutionCtx {
        cancel: CancellationToken::new(),
        project_id,
        pipeline_id: pipeline_id.clone(),
        kind: ExecutionKind::E2e,
        sse_tx: sse_tx.clone(),
        init_payload: crate::server::execution::scheduler::SharedValue::new(init_payload.clone()),
        snapshot_payload: crate::server::execution::scheduler::SharedValue::new(
            init_payload.clone(),
        ),
    });
    state
        .executions
        .write()
        .await
        .insert(execution_id_text.clone(), Arc::clone(&context));

    let (completion_tx, completion) = oneshot::channel();
    let monitor_execution_id = execution_id_text.clone();
    tokio::spawn(monitor_execution(
        queue,
        state,
        context,
        sse_tx,
        init_payload,
        execution_id,
        job_id,
        monitor_execution_id,
        completion_tx,
        config.projection_poll_interval,
    ));

    Ok(StartedE2eExecution {
        execution_id: execution_id_text,
        subscriber,
        completion,
    })
}

pub fn sse_response_for_started_execution(
    started: StartedE2eExecution,
) -> axum::response::Response {
    let (tx, rx) = mpsc::unbounded_channel();
    spawn_broadcast_bridge(started.subscriber, tx, false);
    crate::server::execution::sse_response_from_rx(rx)
}

#[allow(clippy::too_many_arguments)]
async fn monitor_execution(
    queue: QueueRepository,
    state: AppState,
    context: Arc<ExecutionCtx>,
    sse_tx: broadcast::Sender<SseMessage>,
    init_payload: Value,
    execution_id: Uuid,
    job_id: Uuid,
    execution_id_text: String,
    completion: oneshot::Sender<E2eExecutionOutcome>,
    poll_interval: std::time::Duration,
) {
    let _ = send_sse_best_effort(&sse_tx, "execution:init", init_payload);
    let mut last_event_id = 0_i64;
    let mut cancellation_requested = false;
    loop {
        if context.cancel.is_cancelled() && !cancellation_requested {
            let _ = queue.cancel_execution(execution_id).await;
            cancellation_requested = true;
        }
        if let Ok(events) = queue
            .read_events_after(execution_id, last_event_id, 500)
            .await
        {
            for event in events {
                last_event_id = event.id;
                let _ = send_sse_best_effort(&sse_tx, &event.event_type, event.payload_json);
            }
        }

        let terminal = sqlx::query("SELECT status, result_json FROM execution_jobs WHERE id = $1")
            .bind(job_id)
            .fetch_optional(queue.pool())
            .await
            .ok()
            .flatten()
            .and_then(|row| {
                let status: String = row.try_get("status").ok()?;
                matches!(
                    status.as_str(),
                    "completed" | "failed" | "cancelled" | "dead_letter"
                )
                .then_some((
                    status,
                    row.try_get::<Option<Value>, _>("result_json")
                        .ok()
                        .flatten(),
                ))
            });
        if let Some((status, result)) = terminal {
            let snapshot = json!({
                "executionId": execution_id_text,
                "status": status,
                "result": result
            });
            context.snapshot_payload.set(snapshot.clone()).await;
            let _ = send_sse_best_effort(&sse_tx, "execution:status", snapshot);
            state.executions.write().await.remove(&execution_id_text);
            let _ = completion.send(E2eExecutionOutcome {
                execution_id: execution_id_text,
                status,
            });
            return;
        }
        tokio::time::sleep(poll_interval).await;
    }
}
