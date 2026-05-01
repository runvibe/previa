use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::error;

use crate::server::db::{save_load_history, upsert_load_history};
use crate::server::execution::{
    AcquireOutcome, ScheduledExecutionKind, add_load_context_fields,
    build_live_load_snapshot_payload, build_load_snapshot_payload, calculate_node_plan,
    determine_load_history_status, extract_load_context_value, flush_load_batches,
    forward_runner_stream_load_chunked, resolve_runtime_env_groups_for_execution,
    resolve_runtime_specs_for_execution, send_sse_best_effort, snapshot_consolidated_metrics,
    snapshot_latest_lines, split_even,
};
use crate::server::models::{
    HistoryMetadata, LoadEventContext, LoadHistoryWrite, LoadLatencyAccumulator, LoadTestRequest,
    RunnerLoadLine, RunnerLoadPlanItem, SseMessage,
};
use crate::server::state::{
    AppState, EXECUTION_SSE_BUFFER_SIZE, ExecutionCtx, ExecutionKind, LOAD_BATCH_WINDOW_MS,
};
use crate::server::utils::{new_uuid_v7, now_ms};
use crate::server::validation::pipelines::validate_pipeline_templates;

#[derive(Debug)]
pub enum StartLoadExecutionError {
    BadRequest(String),
    ServiceUnavailable(String),
    Internal(String),
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct LoadExecutionOutcome {
    pub execution_id: String,
    pub status: String,
}

pub struct StartedLoadExecution {
    pub execution_id: String,
    pub subscriber: broadcast::Receiver<SseMessage>,
    #[allow(dead_code)]
    pub completion: oneshot::Receiver<LoadExecutionOutcome>,
}

pub async fn start_load_execution(
    state: AppState,
    payload: LoadTestRequest,
    transaction_id: Option<String>,
) -> Result<StartedLoadExecution, StartLoadExecutionError> {
    if payload.pipeline.steps.is_empty() {
        return Err(StartLoadExecutionError::BadRequest(
            "pipeline must contain at least one step".to_owned(),
        ));
    }

    let runner_statuses =
        crate::server::services::runner_registry::collect_registered_runner_statuses(
            &state.db,
            &state.client,
            state.runner_auth_key.as_deref(),
        )
        .await
        .map_err(|err| {
            StartLoadExecutionError::Internal(format!("failed to load runner registry: {err}"))
        })?;
    let registered_nodes: Vec<String> = runner_statuses
        .iter()
        .map(|runner| runner.endpoint.clone())
        .collect();
    let active_nodes: Vec<String> = runner_statuses
        .into_iter()
        .filter(|runner| runner.active)
        .map(|runner| runner.endpoint)
        .collect();
    if active_nodes.is_empty() {
        return Err(StartLoadExecutionError::ServiceUnavailable(
            "No active runners found via /health".to_owned(),
        ));
    }

    let target_rps = (payload.config.concurrency as u64).max(1);

    let plan = calculate_node_plan(
        target_rps,
        state.rps_per_node,
        active_nodes.len(),
        payload.config.total_requests.max(1),
        payload.config.concurrency.max(1),
    );

    let selected_nodes: Vec<String> = active_nodes.iter().take(plan.nodes_used).cloned().collect();
    if selected_nodes.is_empty() {
        return Err(StartLoadExecutionError::ServiceUnavailable(
            "No runner selected for execution".to_owned(),
        ));
    }

    let transaction_id_for_children = transaction_id.clone();
    let history_metadata = HistoryMetadata {
        project_id: payload.project_id.clone(),
        pipeline_index: payload.pipeline_index,
    };
    let runtime_specs = resolve_runtime_specs_for_execution(
        &state.db,
        payload.project_id.as_deref(),
        &payload.specs,
    )
    .await
    .map_err(|err| {
        StartLoadExecutionError::Internal(format!(
            "failed to load project specs for execution: {err}"
        ))
    })?;
    let runtime_env_groups = resolve_runtime_env_groups_for_execution(
        &state.db,
        payload.project_id.as_deref(),
        &payload.env_groups,
    )
    .await
    .map_err(|err| {
        StartLoadExecutionError::Internal(format!(
            "failed to load project env groups for execution: {err}"
        ))
    })?;
    let template_errors = validate_pipeline_templates(
        &payload.pipeline,
        runtime_specs.as_deref(),
        runtime_env_groups.as_deref(),
        payload.selected_env_group_slug.as_deref(),
    );
    if !template_errors.is_empty() {
        return Err(StartLoadExecutionError::BadRequest(
            template_errors.join("; "),
        ));
    }
    let runner_pipeline = payload.pipeline.clone();
    let runner_selected_base_url_key = payload.selected_base_url_key.clone();
    let runner_selected_env_group_slug = payload.selected_env_group_slug.clone();
    let runner_config = payload.config.clone();
    let runner_ramp_up_seconds = runner_config.ramp_up_seconds;
    let history_pipeline_id = payload.pipeline.id.clone();
    let history_pipeline_name = payload.pipeline.name.clone();
    let history_selected_base_url_key = payload.selected_base_url_key.clone();
    let history_request = json!({
        "pipeline": runner_pipeline.clone(),
        "selectedBaseUrlKey": runner_selected_base_url_key.clone(),
        "selectedEnvGroupSlug": runner_selected_env_group_slug.clone(),
        "specs": runtime_specs.clone(),
        "envGroups": runtime_env_groups.clone(),
        "config": runner_config.clone(),
        "projectId": history_metadata.project_id.clone(),
        "pipelineIndex": history_metadata.pipeline_index
    });
    let Some(project_id_for_execution) = payload.project_id.clone() else {
        return Err(StartLoadExecutionError::BadRequest(
            "projectId is required".to_owned(),
        ));
    };
    let orchestrator_execution_id = new_uuid_v7();
    let pipeline_lock_key = load_pipeline_lock_key(
        &project_id_for_execution,
        &payload.pipeline.id,
        payload.pipeline_index,
        &payload.pipeline.name,
    );
    let queue_position = state
        .scheduler
        .enqueue_with_lock(
            orchestrator_execution_id.clone(),
            ScheduledExecutionKind::Load,
            project_id_for_execution.clone(),
            plan.nodes_used.max(1),
            Some(pipeline_lock_key),
        )
        .await;
    let initial_acquire = state
        .scheduler
        .try_acquire(&orchestrator_execution_id, &active_nodes)
        .await;
    let init_payload = match &initial_acquire {
        AcquireOutcome::Reserved(runners) => build_running_load_payload(
            &orchestrator_execution_id,
            &registered_nodes,
            &active_nodes,
            runners,
            &runner_config,
            &plan,
        ),
        AcquireOutcome::Pending { position } => build_queued_load_payload(
            &orchestrator_execution_id,
            &registered_nodes,
            &active_nodes,
            &runner_config,
            &plan,
            *position.max(&queue_position),
        ),
        AcquireOutcome::Missing => build_queued_load_payload(
            &orchestrator_execution_id,
            &registered_nodes,
            &active_nodes,
            &runner_config,
            &plan,
            1,
        ),
    };
    let (sse_tx, _) = broadcast::channel(EXECUTION_SSE_BUFFER_SIZE);
    let response_subscriber = sse_tx.subscribe();
    let init_snapshot = build_load_snapshot_payload(
        &orchestrator_execution_id,
        init_payload
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("queued"),
        extract_load_context_value(&init_payload),
        Vec::new(),
        None,
        Vec::new(),
    );
    let exec_ctx = Arc::new(ExecutionCtx {
        cancel: CancellationToken::new(),
        project_id: project_id_for_execution,
        pipeline_id: history_pipeline_id.clone(),
        kind: ExecutionKind::Load,
        sse_tx: sse_tx.clone(),
        init_payload: crate::server::execution::scheduler::SharedValue::new(init_payload.clone()),
        snapshot_payload: crate::server::execution::scheduler::SharedValue::new(init_snapshot),
    });

    {
        let mut executions = state.executions.write().await;
        executions.insert(orchestrator_execution_id.clone(), Arc::clone(&exec_ctx));
    }

    let state_clone = state.clone();
    let execution_id_for_cleanup = orchestrator_execution_id.clone();
    let history_execution_id = orchestrator_execution_id.clone();
    let runtime_specs_for_runner = runtime_specs.clone().unwrap_or_default();
    let runtime_env_groups_for_runner = runtime_env_groups.clone().unwrap_or_default();
    let (completion_tx, completion_rx) = oneshot::channel();

    tokio::spawn(async move {
        let _ = send_sse_best_effort(&sse_tx, "execution:init", init_payload);

        let (selected_nodes, active_nodes_for_run, emitted_running_status) = match initial_acquire {
            AcquireOutcome::Reserved(runners) => (runners, active_nodes.clone(), false),
            AcquireOutcome::Pending { .. } | AcquireOutcome::Missing => loop {
                if exec_ctx.cancel.is_cancelled() {
                    let _ = state_clone
                        .scheduler
                        .cancel_queued(&history_execution_id)
                        .await;
                    let cancelled_payload = json!({
                        "executionId": history_execution_id,
                        "status": "cancelled",
                        "message": "execution cancelled while queued"
                    });
                    let _ = send_sse_best_effort(&sse_tx, "execution:status", cancelled_payload);
                    let mut executions = state_clone.executions.write().await;
                    executions.remove(&execution_id_for_cleanup);
                    let _ = completion_tx.send(LoadExecutionOutcome {
                        execution_id: history_execution_id,
                        status: "cancelled".to_owned(),
                    });
                    return;
                }

                let runner_statuses =
                    match crate::server::services::runner_registry::collect_registered_runner_statuses(
                        &state_clone.db,
                        &state_clone.client,
                        state_clone.runner_auth_key.as_deref(),
                    )
                    .await
                    {
                        Ok(runner_statuses) => runner_statuses,
                        Err(err) => {
                            error!("failed to load runner registry: {}", err);
                            Vec::new()
                        }
                    };
                let active_nodes = runner_statuses
                    .into_iter()
                    .filter(|runner| runner.active)
                    .map(|runner| runner.endpoint)
                    .collect::<Vec<_>>();
                match state_clone
                    .scheduler
                    .try_acquire(&history_execution_id, &active_nodes)
                    .await
                {
                    AcquireOutcome::Reserved(runners) => break (runners, active_nodes, true),
                    AcquireOutcome::Pending { position } => {
                        let queued_payload = build_queued_load_payload(
                            &history_execution_id,
                            &registered_nodes,
                            &active_nodes,
                            &runner_config,
                            &plan,
                            position,
                        );
                        exec_ctx.init_payload.set(queued_payload.clone()).await;
                        let queued_context = extract_load_context_value(&queued_payload);
                        exec_ctx
                            .snapshot_payload
                            .set(crate::server::execution::build_load_snapshot_payload(
                                &history_execution_id,
                                "queued",
                                queued_context,
                                Vec::new(),
                                None,
                                Vec::new(),
                            ))
                            .await;
                        let _ = send_sse_best_effort(&sse_tx, "execution:status", queued_payload);
                        if !state_clone
                            .scheduler
                            .wait_for_change(&exec_ctx.cancel)
                            .await
                        {
                            continue;
                        }
                    }
                    AcquireOutcome::Missing => {
                        let mut executions = state_clone.executions.write().await;
                        executions.remove(&execution_id_for_cleanup);
                        let _ = completion_tx.send(LoadExecutionOutcome {
                            execution_id: history_execution_id,
                            status: "cancelled".to_owned(),
                        });
                        return;
                    }
                }
            },
        };

        let plan = calculate_node_plan(
            (runner_config.concurrency as u64).max(1),
            state_clone.rps_per_node,
            active_nodes_for_run.len(),
            runner_config.total_requests.max(1),
            runner_config.concurrency.max(1),
        );
        let split_requests = split_even(runner_config.total_requests.max(1), selected_nodes.len());
        let split_concurrency = split_even(runner_config.concurrency.max(1), selected_nodes.len());
        let desired_total_requests = runner_config
            .total_requests
            .max(1)
            .div_ceil(plan.requested_nodes.max(1));
        let runner_load_plan = selected_nodes
            .iter()
            .enumerate()
            .map(|(index, node)| RunnerLoadPlanItem {
                node: node.clone(),
                total_requests: split_requests[index],
                concurrency: split_concurrency[index],
                desired_total_requests,
                above_desired: split_requests[index] > desired_total_requests,
            })
            .collect::<Vec<_>>();
        let overloaded_nodes = runner_load_plan
            .iter()
            .filter(|item| item.above_desired)
            .map(|item| item.node.clone())
            .collect::<Vec<_>>();
        let overloaded_warning = (!overloaded_nodes.is_empty()).then(|| {
            format!(
                "Configured load above desired per-runner totalRequests (desired <= {}): {}.",
                desired_total_requests,
                overloaded_nodes.join(", ")
            )
        });
        let warning = match (plan.warning.clone(), overloaded_warning) {
            (Some(plan_warning), Some(overloaded)) => Some(format!("{plan_warning} {overloaded}")),
            (Some(plan_warning), None) => Some(plan_warning),
            (None, Some(overloaded)) => Some(overloaded),
            (None, None) => None,
        };
        let load_context = Arc::new(LoadEventContext {
            plan: plan.clone(),
            warning,
            registered_nodes: registered_nodes.clone(),
            active_nodes: active_nodes_for_run.clone(),
            used_nodes: selected_nodes.clone(),
            runner_load_plan,
            batch_window_ms: LOAD_BATCH_WINDOW_MS,
        });
        exec_ctx
            .snapshot_payload
            .set(build_live_load_snapshot_payload(
                &history_execution_id,
                "running",
                load_context.as_ref(),
                &[],
                None,
                &[],
            ))
            .await;
        if emitted_running_status {
            let payload = add_load_context_fields(
                json!({ "executionId": history_execution_id, "status": "running" }),
                load_context.as_ref(),
            );
            exec_ctx.init_payload.set(payload.clone()).await;
            let _ = send_sse_best_effort(&sse_tx, "execution:status", payload);
        }

        let started_at_ms = now_ms() as i64;
        let history_record_id = new_uuid_v7();
        let running_context_payload = add_load_context_fields(json!({}), load_context.as_ref());
        let running_requested_config = serde_json::to_value(&runner_config).unwrap_or(Value::Null);
        if let Err(err) = save_load_history(
            &state_clone.db,
            LoadHistoryWrite {
                id: history_record_id.clone(),
                execution_id: history_execution_id.clone(),
                transaction_id: transaction_id.clone(),
                metadata: history_metadata.clone(),
                pipeline_id: history_pipeline_id.clone(),
                pipeline_name: history_pipeline_name.clone(),
                selected_base_url_key: history_selected_base_url_key.clone(),
                status: "running".to_owned(),
                started_at_ms,
                finished_at_ms: started_at_ms,
                duration_ms: 0,
                requested_config: running_requested_config,
                final_consolidated: None,
                final_lines: Vec::new(),
                errors: Vec::new(),
                request: history_request.clone(),
                context: running_context_payload,
            },
        )
        .await
        {
            error!("failed to save load running history: {}", err);
        }

        let load_chunk: Arc<Mutex<HashMap<String, RunnerLoadLine>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let load_latest: Arc<Mutex<HashMap<String, RunnerLoadLine>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let load_latency: Arc<Mutex<LoadLatencyAccumulator>> =
            Arc::new(Mutex::new(LoadLatencyAccumulator::default()));
        let load_errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        let flush_stop = CancellationToken::new();
        let flush_handle = tokio::spawn(flush_load_batches(
            history_execution_id.clone(),
            sse_tx.clone(),
            exec_ctx.cancel.clone(),
            flush_stop.clone(),
            Arc::clone(&load_chunk),
            Arc::clone(&load_latest),
            Arc::clone(&load_latency),
            Arc::clone(&load_errors),
            Arc::clone(&load_context),
            exec_ctx.snapshot_payload.clone(),
        ));

        let mut handles = Vec::with_capacity(selected_nodes.len());
        for (index, node) in selected_nodes.iter().enumerate() {
            let node = node.clone();
            let client = state_clone.client.clone();
            let cancel = exec_ctx.cancel.clone();
            let execution_id = history_execution_id.clone();
            let snapshot_payload = exec_ctx.snapshot_payload.clone();
            let tx = sse_tx.clone();
            let load_chunk = Arc::clone(&load_chunk);
            let load_latest = Arc::clone(&load_latest);
            let load_latency = Arc::clone(&load_latency);
            let load_errors = Arc::clone(&load_errors);
            let load_context = Arc::clone(&load_context);
            let selected_base_url_key = runner_selected_base_url_key.clone();
            let selected_env_group_slug = runner_selected_env_group_slug.clone();
            let pipeline = runner_pipeline.clone();
            let transaction_id = transaction_id_for_children.clone();
            let specs = runtime_specs_for_runner.clone();
            let env_groups = runtime_env_groups_for_runner.clone();
            let runner_auth_key = state_clone.runner_auth_key.clone();

            let child_request = json!({
                "pipeline": pipeline,
                "selectedBaseUrlKey": selected_base_url_key,
                "selectedEnvGroupSlug": selected_env_group_slug,
                "specs": specs,
                "envGroups": env_groups,
                "config": {
                    "totalRequests": split_requests[index],
                    "concurrency": split_concurrency[index],
                    "rampUpSeconds": runner_ramp_up_seconds
                }
            });

            handles.push(tokio::spawn(async move {
                forward_runner_stream_load_chunked(
                    &client,
                    node,
                    child_request,
                    tx,
                    cancel,
                    load_chunk,
                    load_latest,
                    load_latency,
                    load_errors,
                    load_context,
                    execution_id,
                    snapshot_payload,
                    "/api/v1/tests/load",
                    transaction_id,
                    runner_auth_key.as_deref(),
                )
                .await;
            }));
        }

        for handle in handles {
            if let Err(err) = handle.await {
                error!("runner stream task failed: {}", err);
            }
        }

        flush_stop.cancel();
        let _ = flush_handle.await;

        if !exec_ctx.cancel.is_cancelled() {
            let lines = crate::server::execution::drain_load_chunk(&load_chunk).await;
            let consolidated = snapshot_consolidated_metrics(&load_latest, &load_latency).await;
            let payload = add_load_context_fields(
                json!({ "lines": lines, "consolidated": consolidated }),
                load_context.as_ref(),
            );
            let _ = send_sse_best_effort(&sse_tx, "complete", payload);
        }

        let finished_at_ms = now_ms() as i64;
        let duration_ms = finished_at_ms.saturating_sub(started_at_ms);
        let final_lines = snapshot_latest_lines(&load_latest).await;
        let final_consolidated = snapshot_consolidated_metrics(&load_latest, &load_latency).await;
        let errors = load_errors.lock().await.clone();
        let status = determine_load_history_status(
            exec_ctx.cancel.is_cancelled(),
            final_consolidated.as_ref(),
            errors.is_empty(),
        );
        let context_payload = add_load_context_fields(json!({}), load_context.as_ref());
        exec_ctx
            .snapshot_payload
            .set(build_live_load_snapshot_payload(
                &history_execution_id,
                &status,
                load_context.as_ref(),
                &final_lines,
                final_consolidated.as_ref(),
                &errors,
            ))
            .await;

        if let Err(err) = upsert_load_history(
            &state_clone.db,
            LoadHistoryWrite {
                id: history_record_id,
                execution_id: history_execution_id.clone(),
                transaction_id,
                metadata: history_metadata,
                pipeline_id: history_pipeline_id,
                pipeline_name: history_pipeline_name,
                selected_base_url_key: history_selected_base_url_key,
                status: status.clone(),
                started_at_ms,
                finished_at_ms,
                duration_ms,
                requested_config: serde_json::to_value(runner_config).unwrap_or(Value::Null),
                final_consolidated: final_consolidated
                    .and_then(|value| serde_json::to_value(value).ok()),
                final_lines: final_lines
                    .into_iter()
                    .map(|line| serde_json::to_value(line).unwrap_or(Value::Null))
                    .collect(),
                errors,
                request: history_request,
                context: context_payload,
            },
        )
        .await
        {
            error!("failed to save load history: {}", err);
        }

        state_clone.scheduler.release(&history_execution_id).await;
        let mut executions = state_clone.executions.write().await;
        executions.remove(&execution_id_for_cleanup);
        let _ = completion_tx.send(LoadExecutionOutcome {
            execution_id: history_execution_id,
            status,
        });
    });

    Ok(StartedLoadExecution {
        execution_id: orchestrator_execution_id,
        subscriber: response_subscriber,
        completion: completion_rx,
    })
}

pub fn sse_response_for_started_load_execution(
    started: StartedLoadExecution,
) -> axum::response::Response {
    let (tx, rx) = mpsc::unbounded_channel();
    crate::server::execution::spawn_broadcast_bridge(started.subscriber, tx, false);
    crate::server::execution::sse_response_from_rx(rx)
}

fn build_running_load_payload(
    execution_id: &str,
    registered_nodes: &[String],
    active_nodes: &[String],
    used_nodes: &[String],
    config: &crate::server::models::LoadTestConfig,
    plan: &crate::server::models::NodePlan,
) -> Value {
    let runner_load_plan = build_runner_load_plan(config, used_nodes, plan.requested_nodes);
    add_load_context_fields(
        json!({
            "executionId": execution_id,
            "status": "running"
        }),
        &LoadEventContext {
            plan: plan.clone(),
            warning: None,
            registered_nodes: registered_nodes.to_vec(),
            active_nodes: active_nodes.to_vec(),
            used_nodes: used_nodes.to_vec(),
            runner_load_plan,
            batch_window_ms: LOAD_BATCH_WINDOW_MS,
        },
    )
}

fn build_queued_load_payload(
    execution_id: &str,
    registered_nodes: &[String],
    active_nodes: &[String],
    config: &crate::server::models::LoadTestConfig,
    plan: &crate::server::models::NodePlan,
    queue_position: usize,
) -> Value {
    add_load_context_fields(
        json!({
            "executionId": execution_id,
            "status": "queued",
            "queuePosition": queue_position,
            "message": "execution queued waiting for scheduler capacity"
        }),
        &LoadEventContext {
            plan: crate::server::models::NodePlan {
                requested_nodes: plan.requested_nodes,
                nodes_found: plan.nodes_found,
                nodes_used: 0,
                warning: plan.warning.clone(),
            },
            warning: None,
            registered_nodes: registered_nodes.to_vec(),
            active_nodes: active_nodes.to_vec(),
            used_nodes: Vec::new(),
            runner_load_plan: build_runner_load_plan(config, &[], plan.requested_nodes),
            batch_window_ms: LOAD_BATCH_WINDOW_MS,
        },
    )
}

fn build_runner_load_plan(
    config: &crate::server::models::LoadTestConfig,
    used_nodes: &[String],
    requested_nodes: usize,
) -> Vec<RunnerLoadPlanItem> {
    if used_nodes.is_empty() {
        return Vec::new();
    }
    let split_requests = split_even(config.total_requests.max(1), used_nodes.len());
    let split_concurrency = split_even(config.concurrency.max(1), used_nodes.len());
    let desired_total_requests = config
        .total_requests
        .max(1)
        .div_ceil(requested_nodes.max(1));
    used_nodes
        .iter()
        .enumerate()
        .map(|(index, node)| RunnerLoadPlanItem {
            node: node.clone(),
            total_requests: split_requests[index],
            concurrency: split_concurrency[index],
            desired_total_requests,
            above_desired: split_requests[index] > desired_total_requests,
        })
        .collect()
}

fn load_pipeline_lock_key(
    project_id: &str,
    pipeline_id: &Option<String>,
    pipeline_index: Option<i64>,
    pipeline_name: &str,
) -> String {
    if let Some(pipeline_id) = pipeline_id.as_deref() {
        return format!("project:{project_id}:pipeline-id:{pipeline_id}");
    }
    if let Some(pipeline_index) = pipeline_index {
        return format!("project:{project_id}:pipeline-index:{pipeline_index}");
    }
    format!("project:{project_id}:pipeline-name:{pipeline_name}")
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::convert::Infallible;
    use std::sync::Arc;
    use std::time::Duration;

    use axum::body::{Body, Bytes};
    use axum::extract::State;
    use axum::http::{StatusCode, header};
    use axum::response::{IntoResponse, Response};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use previa_runner::{Pipeline, PipelineStep};
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use tokio::sync::{RwLock, mpsc};
    use tokio_stream::wrappers::ReceiverStream;

    use super::start_load_execution;
    use crate::server::execution::ExecutionScheduler;
    use crate::server::models::{LoadTestConfig, LoadTestRequest};
    use crate::server::state::AppState;

    #[tokio::test]
    async fn second_load_execution_is_marked_queued_when_runner_capacity_is_busy() {
        let runner = spawn_busy_runner().await;
        let db = crate::server::db::DbPool::connect("sqlite::memory:", 1)
            .await
            .expect("sqlite memory db");
        sqlx::migrate!("./migrations/sqlite")
            .run(db.pool())
            .await
            .expect("migrations");
        crate::server::db::seed_env_runner_records(&db, &[runner])
            .await
            .expect("seed runner");

        let state = AppState {
            client: reqwest::Client::new(),
            db,
            context_name: "test".to_owned(),
            runner_auth_key: None,
            rps_per_node: 1,
            scheduler: ExecutionScheduler::new(Default::default()),
            executions: Arc::new(RwLock::new(HashMap::new())),
            e2e_queues: Arc::new(RwLock::new(HashMap::new())),
            mcp_sessions: Arc::new(RwLock::new(HashMap::new())),
        };

        let first = start_load_execution(
            state.clone(),
            LoadTestRequest {
                pipeline: test_pipeline("pipe-1"),
                config: test_config(),
                selected_base_url_key: None,
                selected_env_group_slug: None,
                project_id: Some("project-1".to_owned()),
                pipeline_index: Some(0),
                specs: Vec::new(),
                env_groups: Vec::new(),
            },
            None,
        )
        .await
        .expect("first execution");
        let second = start_load_execution(
            state.clone(),
            LoadTestRequest {
                pipeline: test_pipeline("pipe-1"),
                config: test_config(),
                selected_base_url_key: None,
                selected_env_group_slug: None,
                project_id: Some("project-1".to_owned()),
                pipeline_index: Some(0),
                specs: Vec::new(),
                env_groups: Vec::new(),
            },
            None,
        )
        .await
        .expect("second execution");

        let init_payload = {
            let executions = state.executions.read().await;
            executions
                .get(&second.execution_id)
                .expect("second execution context")
                .init_payload
                .get()
                .await
        };
        assert_eq!(init_payload["status"], json!("queued"));
        assert_eq!(init_payload["queuePosition"], json!(1));

        {
            let executions = state.executions.read().await;
            executions
                .get(&first.execution_id)
                .expect("first execution context")
                .cancel
                .cancel();
            executions
                .get(&second.execution_id)
                .expect("second execution context")
                .cancel
                .cancel();
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn second_load_execution_for_same_pipeline_is_queued_even_with_free_runner_capacity() {
        let first_runner = spawn_busy_runner().await;
        let second_runner = spawn_busy_runner().await;
        let db = crate::server::db::DbPool::connect("sqlite::memory:", 1)
            .await
            .expect("sqlite memory db");
        sqlx::migrate!("./migrations/sqlite")
            .run(db.pool())
            .await
            .expect("migrations");
        crate::server::db::seed_env_runner_records(&db, &[first_runner, second_runner])
            .await
            .expect("seed runners");

        let state = AppState {
            client: reqwest::Client::new(),
            db,
            context_name: "test".to_owned(),
            runner_auth_key: None,
            rps_per_node: 1,
            scheduler: ExecutionScheduler::new(Default::default()),
            executions: Arc::new(RwLock::new(HashMap::new())),
            e2e_queues: Arc::new(RwLock::new(HashMap::new())),
            mcp_sessions: Arc::new(RwLock::new(HashMap::new())),
        };

        let first = start_load_execution(
            state.clone(),
            LoadTestRequest {
                pipeline: test_pipeline("pipe-1"),
                config: test_config(),
                selected_base_url_key: None,
                selected_env_group_slug: None,
                project_id: Some("project-1".to_owned()),
                pipeline_index: Some(0),
                specs: Vec::new(),
                env_groups: Vec::new(),
            },
            None,
        )
        .await
        .expect("first execution");
        let second = start_load_execution(
            state.clone(),
            LoadTestRequest {
                pipeline: test_pipeline("pipe-1"),
                config: test_config(),
                selected_base_url_key: None,
                selected_env_group_slug: None,
                project_id: Some("project-1".to_owned()),
                pipeline_index: Some(0),
                specs: Vec::new(),
                env_groups: Vec::new(),
            },
            None,
        )
        .await
        .expect("second execution");

        let init_payload = {
            let executions = state.executions.read().await;
            executions
                .get(&second.execution_id)
                .expect("second execution context")
                .init_payload
                .get()
                .await
        };
        assert_eq!(init_payload["status"], json!("queued"));
        assert_eq!(init_payload["queuePosition"], json!(1));

        {
            let executions = state.executions.read().await;
            executions
                .get(&first.execution_id)
                .expect("first execution context")
                .cancel
                .cancel();
            executions
                .get(&second.execution_id)
                .expect("second execution context")
                .cancel
                .cancel();
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    async fn spawn_busy_runner() -> String {
        async fn health() -> impl IntoResponse {
            Json(json!({ "status": "ok" }))
        }

        async fn load(State(()): State<()>, Json(_payload): Json<Value>) -> Response {
            let (tx, rx) = mpsc::channel::<Result<Bytes, Infallible>>(8);
            tokio::spawn(async move {
                let _ = tx
                    .send(Ok(Bytes::from(
                        "event: execution:init\ndata: {\"status\":\"running\"}\n\n",
                    )))
                    .await;
                tokio::time::sleep(Duration::from_secs(2)).await;
            });

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/event-stream")
                .body(Body::from_stream(ReceiverStream::new(rx)))
                .unwrap()
        }

        let app = Router::new()
            .route("/health", get(health))
            .route("/api/v1/tests/load", post(load))
            .with_state(());

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("runner server");
        });
        format!("http://{}", addr)
    }

    fn test_pipeline(id: &str) -> Pipeline {
        Pipeline {
            id: Some(id.to_owned()),
            name: "Pipeline".to_owned(),
            description: None,
            steps: vec![PipelineStep {
                id: "step-1".to_owned(),
                name: "Step 1".to_owned(),
                description: None,
                method: "GET".to_owned(),
                url: "https://example.com".to_owned(),
                headers: Default::default(),
                body: None,
                operation_id: None,
                delay: None,
                retry: None,
                asserts: Vec::new(),
            }],
        }
    }

    fn test_config() -> LoadTestConfig {
        LoadTestConfig {
            total_requests: 10,
            concurrency: 1,
            ramp_up_seconds: 0.0,
        }
    }
}
