use std::sync::Arc;

use rand::seq::SliceRandom;
use serde_json::json;
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::error;

use crate::server::db::{save_e2e_history, upsert_e2e_history};
use crate::server::execution::{
    AcquireOutcome, ScheduledExecutionKind, add_context_fields, build_e2e_snapshot_payload,
    determine_e2e_history_status, forward_runner_stream, resolve_runtime_env_groups_for_execution,
    resolve_runtime_specs_for_execution, send_sse_best_effort, spawn_broadcast_bridge,
};
use crate::server::models::{
    E2eHistoryAccumulator, E2eHistoryWrite, E2eTestRequest, HistoryMetadata, NodePlan, SseMessage,
};
use crate::server::services::e2e_rerun::{ordered_prior_results, validate_rerun_context};
use crate::server::state::{AppState, EXECUTION_SSE_BUFFER_SIZE, ExecutionCtx, ExecutionKind};
use crate::server::utils::{new_uuid_v7, now_ms};
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
        StartE2eExecutionError::Internal(format!(
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
        StartE2eExecutionError::Internal(format!(
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
        return Err(StartE2eExecutionError::BadRequest(
            template_errors.join("; "),
        ));
    }

    let pipeline_for_runner = payload.pipeline.clone();
    let pipeline_id = payload.pipeline.id.clone();
    let pipeline_name = payload.pipeline.name.clone();
    let selected_base_url_key = payload.selected_base_url_key.clone();
    let selected_base_url_key_for_runner = payload.selected_base_url_key.clone();
    let selected_env_group_slug_for_runner = payload.selected_env_group_slug.clone();
    let start_step_id_for_runner = payload.start_step_id.clone();
    let prior_results_for_runner = payload.prior_results.clone();
    let history_request = json!({
        "pipeline": payload.pipeline,
        "selectedBaseUrlKey": payload.selected_base_url_key,
        "selectedEnvGroupSlug": payload.selected_env_group_slug,
        "startStepId": payload.start_step_id,
        "priorResults": payload.prior_results,
        "specs": runtime_specs.clone(),
        "envGroups": runtime_env_groups.clone(),
        "projectId": payload.project_id,
        "pipelineIndex": payload.pipeline_index
    });
    let Some(project_id_for_execution) = payload.project_id.clone() else {
        return Err(StartE2eExecutionError::BadRequest(
            "projectId is required".to_owned(),
        ));
    };

    let orchestrator_execution_id = new_uuid_v7();
    let (sse_tx, _) = broadcast::channel(EXECUTION_SSE_BUFFER_SIZE);
    let response_subscriber = sse_tx.subscribe();
    let mut active_nodes =
        crate::server::services::runner_registry::collect_active_registered_runner_endpoints(
            &state.db,
            &state.client,
            state.runner_auth_key.as_deref(),
        )
        .await
        .map_err(|err| {
            StartE2eExecutionError::Internal(format!("failed to load runner registry: {err}"))
        })?;
    active_nodes.shuffle(&mut rand::rng());
    if active_nodes.is_empty() {
        return Err(StartE2eExecutionError::ServiceUnavailable(
            "No active runners found via /health".to_owned(),
        ));
    }
    let queue_position = state
        .scheduler
        .enqueue(
            orchestrator_execution_id.clone(),
            ScheduledExecutionKind::E2e,
            project_id_for_execution.clone(),
            1,
        )
        .await;
    let initial_acquire = state
        .scheduler
        .try_acquire(&orchestrator_execution_id, &active_nodes)
        .await;
    let init_payload = match &initial_acquire {
        AcquireOutcome::Reserved(runners) => {
            running_payload(&orchestrator_execution_id, runners, active_nodes.len())
        }
        AcquireOutcome::Pending { position } => queued_payload(
            &orchestrator_execution_id,
            active_nodes.len(),
            *position.max(&queue_position),
        ),
        AcquireOutcome::Missing => {
            queued_payload(&orchestrator_execution_id, active_nodes.len(), 1)
        }
    };
    let initial_history = E2eHistoryAccumulator {
        steps: start_step_id_for_runner
            .as_deref()
            .map(|start_step_id| {
                ordered_prior_results(
                    &pipeline_for_runner,
                    start_step_id,
                    &prior_results_for_runner,
                )
            })
            .unwrap_or_default(),
        ..E2eHistoryAccumulator::default()
    };
    let history_accumulator = Arc::new(Mutex::new(initial_history.clone()));
    let exec_ctx = Arc::new(ExecutionCtx {
        cancel: CancellationToken::new(),
        project_id: project_id_for_execution,
        pipeline_id: pipeline_id.clone(),
        kind: ExecutionKind::E2e,
        sse_tx: sse_tx.clone(),
        init_payload: crate::server::execution::scheduler::SharedValue::new(init_payload.clone()),
        snapshot_payload: crate::server::execution::scheduler::SharedValue::new(
            build_e2e_snapshot_payload(
                &orchestrator_execution_id,
                init_payload
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("queued"),
                &initial_history,
            ),
        ),
    });

    {
        let mut executions = state.executions.write().await;
        executions.insert(orchestrator_execution_id.clone(), Arc::clone(&exec_ctx));
    }

    let runtime_specs_for_runner = runtime_specs.clone().unwrap_or_default();
    let runtime_env_groups_for_runner = runtime_env_groups.clone().unwrap_or_default();
    let transaction_id_for_runner = transaction_id.clone();
    let state_clone = state.clone();
    let execution_id_for_cleanup = orchestrator_execution_id.clone();
    let history_execution_id = orchestrator_execution_id.clone();
    let (completion_tx, completion_rx) = oneshot::channel();

    tokio::spawn(async move {
        let _ = send_sse_best_effort(&sse_tx, "execution:init", init_payload.clone());

        let (selected_runners, nodes_found, emitted_running_status) = match initial_acquire {
            AcquireOutcome::Reserved(runners) => (runners, active_nodes.len(), false),
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
                    let _ = completion_tx.send(E2eExecutionOutcome {
                        execution_id: history_execution_id,
                        status: "cancelled".to_owned(),
                    });
                    return;
                }

                let mut active_nodes =
                    match crate::server::services::runner_registry::collect_active_registered_runner_endpoints(
                        &state_clone.db,
                        &state_clone.client,
                        state_clone.runner_auth_key.as_deref(),
                    )
                    .await
                    {
                        Ok(active_nodes) => active_nodes,
                        Err(err) => {
                            error!("failed to load runner registry: {}", err);
                            Vec::new()
                        }
                    };
                active_nodes.shuffle(&mut rand::rng());
                match state_clone
                    .scheduler
                    .try_acquire(&history_execution_id, &active_nodes)
                    .await
                {
                    AcquireOutcome::Reserved(runners) => {
                        break (runners, active_nodes.len(), true);
                    }
                    AcquireOutcome::Pending { position } => {
                        let queued =
                            queued_payload(&history_execution_id, active_nodes.len(), position);
                        exec_ctx.init_payload.set(queued).await;
                        let snapshot = history_accumulator.lock().await.clone();
                        exec_ctx
                            .snapshot_payload
                            .set(build_e2e_snapshot_payload(
                                &history_execution_id,
                                "queued",
                                &snapshot,
                            ))
                            .await;
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
                        let _ = completion_tx.send(E2eExecutionOutcome {
                            execution_id: history_execution_id,
                            status: "cancelled".to_owned(),
                        });
                        return;
                    }
                }
            },
        };

        let selected_node = selected_runners[0].clone();
        let plan = NodePlan {
            requested_nodes: 1,
            nodes_found,
            nodes_used: 1,
            warning: None,
        };
        if emitted_running_status {
            let payload = running_payload(&history_execution_id, &selected_runners, nodes_found);
            exec_ctx.init_payload.set(payload.clone()).await;
            let snapshot = history_accumulator.lock().await.clone();
            exec_ctx
                .snapshot_payload
                .set(build_e2e_snapshot_payload(
                    &history_execution_id,
                    "running",
                    &snapshot,
                ))
                .await;
            let _ = send_sse_best_effort(&sse_tx, "execution:status", payload);
        }

        let started_at_ms = now_ms() as i64;
        let history_record_id = new_uuid_v7();
        if let Err(err) = save_e2e_history(
            &state_clone.db,
            E2eHistoryWrite {
                id: history_record_id.clone(),
                execution_id: history_execution_id.clone(),
                transaction_id: transaction_id.clone(),
                metadata: history_metadata.clone(),
                pipeline_id: pipeline_id.clone(),
                pipeline_name: pipeline_name.clone(),
                selected_base_url_key: selected_base_url_key.clone(),
                status: "running".to_owned(),
                started_at_ms,
                finished_at_ms: started_at_ms,
                duration_ms: 0,
                summary: None,
                steps: Vec::new(),
                errors: Vec::new(),
                request: history_request.clone(),
            },
        )
        .await
        {
            error!("failed to save e2e running history: {}", err);
        }

        let (request_body, endpoint_path) =
            if let Some(start_step_id) = start_step_id_for_runner.clone() {
                (
                    json!({
                        "pipeline": pipeline_for_runner,
                        "startStepId": start_step_id,
                        "priorResults": prior_results_for_runner,
                        "selectedEnvGroupSlug": selected_env_group_slug_for_runner,
                        "specs": runtime_specs_for_runner,
                        "envGroups": runtime_env_groups_for_runner
                    }),
                    "/api/v1/tests/e2e/rerun-from-step",
                )
            } else {
                (
                    json!({
                        "pipeline": pipeline_for_runner,
                        "selectedBaseUrlKey": selected_base_url_key_for_runner,
                        "selectedEnvGroupSlug": selected_env_group_slug_for_runner,
                        "specs": runtime_specs_for_runner,
                        "envGroups": runtime_env_groups_for_runner
                    }),
                    "/api/v1/tests/e2e",
                )
            };

        forward_runner_stream(
            &state_clone.client,
            selected_node,
            request_body,
            sse_tx,
            exec_ctx.cancel.clone(),
            plan,
            endpoint_path,
            transaction_id_for_runner,
            state_clone.runner_auth_key.as_deref(),
            Some((
                history_execution_id.clone(),
                Arc::clone(&history_accumulator),
                exec_ctx.snapshot_payload.clone(),
            )),
        )
        .await;

        let finished_at_ms = now_ms() as i64;
        let duration_ms = finished_at_ms.saturating_sub(started_at_ms);
        let mut snapshot = history_accumulator.lock().await.clone();
        if start_step_id_for_runner.is_some() {
            snapshot.summary = Some(combined_e2e_summary(&snapshot));
        }
        let status = determine_e2e_history_status(exec_ctx.cancel.is_cancelled(), &snapshot);
        exec_ctx
            .snapshot_payload
            .set(build_e2e_snapshot_payload(
                &history_execution_id,
                &status,
                &snapshot,
            ))
            .await;

        if let Err(err) = upsert_e2e_history(
            &state_clone.db,
            E2eHistoryWrite {
                id: history_record_id,
                execution_id: history_execution_id.clone(),
                transaction_id: transaction_id.clone(),
                metadata: history_metadata,
                pipeline_id,
                pipeline_name,
                selected_base_url_key,
                status: status.clone(),
                started_at_ms,
                finished_at_ms,
                duration_ms,
                summary: snapshot.summary,
                steps: snapshot.steps,
                errors: snapshot.errors,
                request: history_request,
            },
        )
        .await
        {
            error!("failed to save e2e history: {}", err);
        }

        state_clone.scheduler.release(&history_execution_id).await;
        let mut executions = state_clone.executions.write().await;
        executions.remove(&execution_id_for_cleanup);
        let _ = completion_tx.send(E2eExecutionOutcome {
            execution_id: history_execution_id,
            status,
        });
    });

    Ok(StartedE2eExecution {
        execution_id: orchestrator_execution_id,
        subscriber: response_subscriber,
        completion: completion_rx,
    })
}

pub fn sse_response_for_started_execution(
    started: StartedE2eExecution,
) -> axum::response::Response {
    let (tx, rx) = mpsc::unbounded_channel();
    spawn_broadcast_bridge(started.subscriber, tx, false);
    crate::server::execution::sse_response_from_rx(rx)
}

fn combined_e2e_summary(snapshot: &E2eHistoryAccumulator) -> serde_json::Value {
    let total_steps = snapshot.steps.len();
    let failed = snapshot
        .steps
        .iter()
        .filter(|step| step.get("status").and_then(serde_json::Value::as_str) == Some("error"))
        .count();
    let total_duration = snapshot
        .steps
        .iter()
        .filter_map(|step| step.get("duration").and_then(serde_json::Value::as_u64))
        .sum::<u64>();

    json!({
        "totalSteps": total_steps,
        "passed": total_steps.saturating_sub(failed),
        "failed": failed,
        "totalDuration": total_duration,
    })
}

fn queued_payload(
    execution_id: &str,
    nodes_found: usize,
    queue_position: usize,
) -> serde_json::Value {
    add_context_fields(
        json!({
            "executionId": execution_id,
            "status": "queued",
            "queuePosition": queue_position,
            "message": "execution queued waiting for scheduler capacity"
        }),
        &[],
        &NodePlan {
            requested_nodes: 1,
            nodes_found,
            nodes_used: 0,
            warning: None,
        },
    )
}

fn running_payload(
    execution_id: &str,
    runners: &[String],
    nodes_found: usize,
) -> serde_json::Value {
    add_context_fields(
        json!({
            "executionId": execution_id,
            "status": "running"
        }),
        runners,
        &NodePlan {
            requested_nodes: 1,
            nodes_found,
            nodes_used: 1,
            warning: None,
        },
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::convert::Infallible;
    use std::sync::Arc;
    use std::time::Duration;

    use axum::Json;
    use axum::body::{Body, Bytes};
    use axum::extract::State;
    use axum::http::{StatusCode, header};
    use axum::response::Response;
    use axum::routing::{get, post};
    use axum::{Router, response::IntoResponse};
    use previa_runner::{Pipeline, PipelineStep};
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use tokio::sync::{RwLock, mpsc};
    use tokio_stream::wrappers::ReceiverStream;

    use super::start_e2e_execution;
    use crate::server::execution::ExecutionScheduler;
    use crate::server::state::AppState;

    #[tokio::test]
    async fn second_e2e_execution_is_marked_queued_when_slot_is_busy() {
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
            auth: crate::server::auth::AuthRuntime::anonymous(),
            rps_per_node: 1,
            scheduler: ExecutionScheduler::new(Default::default()),
            executions: Arc::new(RwLock::new(HashMap::new())),
            e2e_queues: Arc::new(RwLock::new(HashMap::new())),
            mcp_sessions: Arc::new(RwLock::new(HashMap::new())),
        };

        let first = start_e2e_execution(
            state.clone(),
            crate::server::models::E2eTestRequest {
                pipeline: test_pipeline("pipe-1"),
                selected_base_url_key: None,
                selected_env_group_slug: None,
                project_id: Some("project-1".to_owned()),
                pipeline_index: Some(0),
                start_step_id: None,
                prior_results: HashMap::new(),
                specs: Vec::new(),
                env_groups: Vec::new(),
            },
            None,
        )
        .await
        .expect("first execution");
        let second = start_e2e_execution(
            state.clone(),
            crate::server::models::E2eTestRequest {
                pipeline: test_pipeline("pipe-1"),
                selected_base_url_key: None,
                selected_env_group_slug: None,
                project_id: Some("project-1".to_owned()),
                pipeline_index: Some(0),
                start_step_id: None,
                prior_results: HashMap::new(),
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

        async fn e2e(State(()): State<()>, Json(_payload): Json<Value>) -> Response {
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
            .route("/api/v1/tests/e2e", post(e2e))
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
}
