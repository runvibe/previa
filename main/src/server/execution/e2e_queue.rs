use std::sync::Arc;
use std::time::Duration;

use axum::response::{IntoResponse, Response};
use serde_json::json;
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::server::db::{
    cancel_non_terminal_e2e_queue, insert_e2e_queue, load_e2e_queue_record,
    load_existing_project_pipeline_ids, load_project_pipeline_for_execution, queue_request_json,
    update_e2e_queue_item_status, update_e2e_queue_status,
};
use crate::server::errors::{
    bad_request_message_response, internal_error_response, not_found_response,
};
use crate::server::execution::{
    StartE2eExecutionError, spawn_broadcast_bridge, sse_response_from_rx, start_e2e_execution,
};
use crate::server::models::{
    E2eQueueRecord, E2eQueueStatus, E2eTestRequest, ProjectE2eQueueRequest, SseMessage,
};
use crate::server::state::{AppState, E2eQueueRuntime};
use crate::server::utils::{new_uuid_v7, now_iso};

#[derive(Debug)]
pub enum QueueError {
    BadRequest(String),
    NotFound(String),
    Internal(String),
}

pub async fn create_e2e_queue(
    state: AppState,
    project_id: String,
    request: ProjectE2eQueueRequest,
) -> Result<E2eQueueRecord, QueueError> {
    let pipeline_ids = request
        .pipeline_ids
        .iter()
        .map(|pipeline_id| pipeline_id.trim().to_owned())
        .collect::<Vec<_>>();
    if pipeline_ids.is_empty()
        || pipeline_ids
            .iter()
            .any(|pipeline_id| pipeline_id.is_empty())
    {
        return Err(QueueError::BadRequest(
            "pipelineIds must contain at least one non-empty value".to_owned(),
        ));
    }

    let existing = load_existing_project_pipeline_ids(&state.db, &project_id, &pipeline_ids)
        .await
        .map_err(|err| QueueError::Internal(format!("failed to validate pipelines: {err}")))?;
    let missing = pipeline_ids
        .iter()
        .filter(|pipeline_id| !existing.contains(*pipeline_id))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(QueueError::BadRequest(format!(
            "unknown pipelineIds for project: {}",
            missing.join(", ")
        )));
    }

    let queue_id = new_uuid_v7();
    let created_at = now_iso();
    let request_json = queue_request_json(
        &pipeline_ids,
        request.selected_base_url_key.as_deref(),
        request.selected_env_group_slug.as_deref(),
        &request.specs,
        &request.env_groups,
    );
    let snapshot = insert_e2e_queue(
        &state.db,
        &project_id,
        &queue_id,
        request.selected_base_url_key.as_deref(),
        &request_json,
        &pipeline_ids,
        &created_at,
    )
    .await
    .map_err(|err| QueueError::Internal(format!("failed to persist e2e queue: {err}")))?;

    let runtime = E2eQueueRuntime::new(queue_id.clone(), project_id.clone(), snapshot);
    let previous = {
        let mut queues = state.e2e_queues.write().await;
        queues.insert(project_id.clone(), Arc::clone(&runtime))
    };

    if let Some(previous_runtime) = previous.as_ref() {
        previous_runtime.cancel.cancel();
        if let Some(execution_id) = previous_runtime.active_execution_id().await {
            cancel_child_execution(&state, &execution_id).await;
        }
    }

    tokio::spawn(run_e2e_queue(
        state,
        project_id,
        request,
        Arc::clone(&runtime),
        previous,
    ));

    Ok(E2eQueueRecord {
        id: queue_id,
        ..runtime.snapshot().await
    })
}

pub async fn get_e2e_queue_response(
    state: AppState,
    project_id: String,
    queue_id: String,
) -> Result<Response, QueueError> {
    if let Some(snapshot) = get_e2e_queue_snapshot(&state, &project_id, &queue_id).await? {
        let (tx, rx) = mpsc::unbounded_channel::<SseMessage>();
        let _ = tx.send(SseMessage {
            event: "queue:update".to_owned(),
            data: serde_json::to_value(&snapshot).unwrap_or_else(|_| json!({})),
        });
        if let Some(runtime) = active_queue_runtime(&state, &project_id, &queue_id).await {
            spawn_broadcast_bridge(runtime.sse_tx.subscribe(), tx, false);
            return Ok(sse_response_from_rx(rx));
        }

        return Ok(axum::Json(snapshot).into_response());
    }

    Err(QueueError::NotFound("e2e queue not found".to_owned()))
}

pub async fn get_current_e2e_queue_response(
    state: AppState,
    project_id: String,
) -> Result<Response, QueueError> {
    Ok(axum::Json(get_current_e2e_queue_snapshot(&state, &project_id).await?).into_response())
}

pub async fn cancel_e2e_queue(
    state: AppState,
    project_id: String,
    queue_id: String,
) -> Result<(), QueueError> {
    let snapshot = load_e2e_queue_record(&state.db, &project_id, &queue_id)
        .await
        .map_err(|err| QueueError::Internal(format!("failed to load e2e queue: {err}")))?;
    let Some(snapshot) = snapshot else {
        return Err(QueueError::NotFound("e2e queue not found".to_owned()));
    };
    if snapshot.status.is_terminal() {
        return Ok(());
    }

    if let Some(runtime) = active_queue_runtime(&state, &project_id, &queue_id).await {
        runtime.cancel.cancel();
        if let Some(execution_id) = runtime.active_execution_id().await {
            cancel_child_execution(&state, &execution_id).await;
        }
        return Ok(());
    }

    let updated_at = now_iso();
    cancel_non_terminal_e2e_queue(&state.db, &queue_id, &updated_at)
        .await
        .map_err(|err| QueueError::Internal(format!("failed to cancel e2e queue: {err}")))?;
    Ok(())
}

pub fn queue_error_response(err: QueueError) -> Response {
    match err {
        QueueError::BadRequest(message) => bad_request_message_response(&message),
        QueueError::NotFound(message) => not_found_response(&message),
        QueueError::Internal(message) => internal_error_response(message),
    }
}

pub async fn get_e2e_queue_snapshot(
    state: &AppState,
    project_id: &str,
    queue_id: &str,
) -> Result<Option<E2eQueueRecord>, QueueError> {
    if let Some(runtime) = active_queue_runtime(state, project_id, queue_id).await {
        return Ok(Some(runtime.snapshot().await));
    }

    load_e2e_queue_record(&state.db, project_id, queue_id)
        .await
        .map_err(|err| QueueError::Internal(format!("failed to load e2e queue: {err}")))
}

pub async fn get_current_e2e_queue_snapshot(
    state: &AppState,
    project_id: &str,
) -> Result<E2eQueueRecord, QueueError> {
    let runtime = {
        let queues = state.e2e_queues.read().await;
        queues.get(project_id).cloned()
    };

    let Some(runtime) = runtime else {
        return Err(QueueError::NotFound(
            "no active e2e queue for project".to_owned(),
        ));
    };

    Ok(runtime.snapshot().await)
}

async fn active_queue_runtime(
    state: &AppState,
    project_id: &str,
    queue_id: &str,
) -> Option<Arc<E2eQueueRuntime>> {
    let queues = state.e2e_queues.read().await;
    queues
        .get(project_id)
        .filter(|runtime| runtime.queue_id == queue_id)
        .cloned()
}

async fn run_e2e_queue(
    state: AppState,
    project_id: String,
    request: ProjectE2eQueueRequest,
    runtime: Arc<E2eQueueRuntime>,
    previous: Option<Arc<E2eQueueRuntime>>,
) {
    if let Some(previous) = previous {
        previous.wait_finished().await;
    }

    let mut snapshot = runtime.snapshot().await;
    runtime.set_snapshot(snapshot.clone()).await;

    let mut terminal_status = E2eQueueStatus::Completed;

    if !wait_for_project_executions_to_finish(&state, &project_id, &runtime).await {
        terminal_status = E2eQueueStatus::Cancelled;
    }

    for position in 0..snapshot.pipelines.len() {
        if terminal_status == E2eQueueStatus::Cancelled {
            break;
        }
        if runtime.cancel.is_cancelled() {
            terminal_status = E2eQueueStatus::Cancelled;
            break;
        }

        let pipeline_id = snapshot.pipelines[position].id.clone();
        let started_at = now_iso();
        snapshot.status = E2eQueueStatus::Running;
        snapshot.updated_at = started_at.clone();
        snapshot.pipelines[position].status = E2eQueueStatus::Running;
        snapshot.pipelines[position].updated_at = started_at.clone();

        if let Err(err) = update_e2e_queue_status(
            &state.db,
            &runtime.queue_id,
            E2eQueueStatus::Running,
            &started_at,
            None,
        )
        .await
        {
            finalize_runtime_with_internal_error(&state, &runtime, &mut snapshot, err).await;
            return;
        }
        if let Err(err) = update_e2e_queue_item_status(
            &state.db,
            &runtime.queue_id,
            position,
            E2eQueueStatus::Running,
            &started_at,
            None,
        )
        .await
        {
            finalize_runtime_with_internal_error(&state, &runtime, &mut snapshot, err).await;
            return;
        }
        runtime.set_snapshot(snapshot.clone()).await;

        let pipeline =
            match load_project_pipeline_for_execution(&state.db, &project_id, &pipeline_id).await {
                Ok(Some((pipeline, pipeline_index))) => (pipeline, pipeline_index),
                Ok(None) => {
                    mark_queue_failed(&state, &runtime, &mut snapshot, position, None).await;
                    return;
                }
                Err(err) => {
                    finalize_runtime_with_internal_error(&state, &runtime, &mut snapshot, err)
                        .await;
                    return;
                }
            };

        let start_result = start_e2e_execution(
            state.clone(),
            E2eTestRequest {
                pipeline: pipeline.0,
                selected_base_url_key: request.selected_base_url_key.clone(),
                selected_env_group_slug: request.selected_env_group_slug.clone(),
                project_id: Some(project_id.clone()),
                pipeline_index: Some(pipeline.1),
                start_step_id: None,
                prior_results: Default::default(),
                specs: request.specs.clone(),
                env_groups: request.env_groups.clone(),
            },
            None,
        )
        .await;

        let started = match start_result {
            Ok(started) => started,
            Err(StartE2eExecutionError::BadRequest(_))
            | Err(StartE2eExecutionError::ServiceUnavailable(_))
            | Err(StartE2eExecutionError::Internal(_)) => {
                mark_queue_failed(&state, &runtime, &mut snapshot, position, None).await;
                return;
            }
        };

        runtime
            .set_active_execution_id(Some(started.execution_id.clone()))
            .await;
        if let Err(err) = update_e2e_queue_status(
            &state.db,
            &runtime.queue_id,
            E2eQueueStatus::Running,
            &snapshot.updated_at,
            Some(&started.execution_id),
        )
        .await
        {
            finalize_runtime_with_internal_error(&state, &runtime, &mut snapshot, err).await;
            return;
        }
        if let Err(err) = update_e2e_queue_item_status(
            &state.db,
            &runtime.queue_id,
            position,
            E2eQueueStatus::Running,
            &snapshot.pipelines[position].updated_at,
            Some(&started.execution_id),
        )
        .await
        {
            finalize_runtime_with_internal_error(&state, &runtime, &mut snapshot, err).await;
            return;
        }

        let completion = started.completion;
        tokio::pin!(completion);
        let completion = tokio::select! {
            _ = runtime.cancel.cancelled() => {
                cancel_child_execution(&state, &started.execution_id).await;
                let _ = timeout(Duration::from_secs(5), &mut completion).await;
                terminal_status = E2eQueueStatus::Cancelled;
                break;
            }
            completion = &mut completion => completion.ok(),
        };

        runtime.set_active_execution_id(None).await;

        let finished_at = now_iso();
        snapshot.updated_at = finished_at.clone();
        let item = &mut snapshot.pipelines[position];
        item.updated_at = finished_at.clone();

        match completion {
            Some(outcome) if outcome.status == "success" => {
                item.status = E2eQueueStatus::Completed;
                if let Err(err) = update_e2e_queue_item_status(
                    &state.db,
                    &runtime.queue_id,
                    position,
                    E2eQueueStatus::Completed,
                    &finished_at,
                    Some(&outcome.execution_id),
                )
                .await
                {
                    finalize_runtime_with_internal_error(&state, &runtime, &mut snapshot, err)
                        .await;
                    return;
                }

                let next_status = if position + 1 == snapshot.pipelines.len() {
                    E2eQueueStatus::Completed
                } else {
                    E2eQueueStatus::Running
                };
                snapshot.status = next_status;
                if let Err(err) = update_e2e_queue_status(
                    &state.db,
                    &runtime.queue_id,
                    next_status,
                    &finished_at,
                    None,
                )
                .await
                {
                    finalize_runtime_with_internal_error(&state, &runtime, &mut snapshot, err)
                        .await;
                    return;
                }
                runtime.set_snapshot(snapshot.clone()).await;
            }
            Some(outcome) if outcome.status == "cancelled" && runtime.cancel.is_cancelled() => {
                terminal_status = E2eQueueStatus::Cancelled;
                break;
            }
            Some(outcome) => {
                mark_queue_failed(
                    &state,
                    &runtime,
                    &mut snapshot,
                    position,
                    Some(outcome.execution_id),
                )
                .await;
                return;
            }
            None => {
                mark_queue_failed(&state, &runtime, &mut snapshot, position, None).await;
                return;
            }
        }
    }

    let final_updated_at = now_iso();
    snapshot.updated_at = final_updated_at.clone();
    snapshot.status = terminal_status;

    if terminal_status == E2eQueueStatus::Cancelled {
        for (position, item) in snapshot.pipelines.iter_mut().enumerate() {
            if matches!(
                item.status,
                E2eQueueStatus::Pending | E2eQueueStatus::Running
            ) {
                item.status = E2eQueueStatus::Cancelled;
                item.updated_at = final_updated_at.clone();
                let _ = update_e2e_queue_item_status(
                    &state.db,
                    &runtime.queue_id,
                    position,
                    E2eQueueStatus::Cancelled,
                    &final_updated_at,
                    None,
                )
                .await;
            }
        }
    }

    let _ = update_e2e_queue_status(
        &state.db,
        &runtime.queue_id,
        terminal_status,
        &final_updated_at,
        None,
    )
    .await;
    runtime.set_snapshot(snapshot.clone()).await;
    finish_runtime(&state, &runtime).await;
}

async fn mark_queue_failed(
    state: &AppState,
    runtime: &Arc<E2eQueueRuntime>,
    snapshot: &mut E2eQueueRecord,
    failed_position: usize,
    execution_id: Option<String>,
) {
    let updated_at = now_iso();
    snapshot.status = E2eQueueStatus::Failed;
    snapshot.updated_at = updated_at.clone();
    snapshot.pipelines[failed_position].status = E2eQueueStatus::Failed;
    snapshot.pipelines[failed_position].updated_at = updated_at.clone();

    let _ = update_e2e_queue_item_status(
        &state.db,
        &runtime.queue_id,
        failed_position,
        E2eQueueStatus::Failed,
        &updated_at,
        execution_id.as_deref(),
    )
    .await;

    for (position, item) in snapshot
        .pipelines
        .iter_mut()
        .enumerate()
        .skip(failed_position + 1)
    {
        if item.status == E2eQueueStatus::Pending {
            item.status = E2eQueueStatus::Cancelled;
            item.updated_at = updated_at.clone();
            let _ = update_e2e_queue_item_status(
                &state.db,
                &runtime.queue_id,
                position,
                E2eQueueStatus::Cancelled,
                &updated_at,
                None,
            )
            .await;
        }
    }

    let _ = update_e2e_queue_status(
        &state.db,
        &runtime.queue_id,
        E2eQueueStatus::Failed,
        &updated_at,
        None,
    )
    .await;
    runtime.set_active_execution_id(None).await;
    runtime.set_snapshot(snapshot.clone()).await;
    finish_runtime(state, runtime).await;
}

async fn finalize_runtime_with_internal_error(
    state: &AppState,
    runtime: &Arc<E2eQueueRuntime>,
    snapshot: &mut E2eQueueRecord,
    err: sqlx::Error,
) {
    let updated_at = now_iso();
    snapshot.status = E2eQueueStatus::Failed;
    snapshot.updated_at = updated_at.clone();
    for item in &mut snapshot.pipelines {
        if item.status == E2eQueueStatus::Pending {
            item.status = E2eQueueStatus::Cancelled;
            item.updated_at = updated_at.clone();
        }
    }
    let _ = update_e2e_queue_status(
        &state.db,
        &runtime.queue_id,
        E2eQueueStatus::Failed,
        &updated_at,
        None,
    )
    .await;
    runtime.set_active_execution_id(None).await;
    runtime.set_snapshot(snapshot.clone()).await;
    tracing::error!("failed to persist e2e queue state: {}", err);
    finish_runtime(state, runtime).await;
}

async fn finish_runtime(state: &AppState, runtime: &Arc<E2eQueueRuntime>) {
    runtime.mark_finished();
    let mut queues = state.e2e_queues.write().await;
    if queues
        .get(&runtime.project_id)
        .is_some_and(|current| current.queue_id == runtime.queue_id)
    {
        queues.remove(&runtime.project_id);
    }
}

async fn cancel_child_execution(state: &AppState, execution_id: &str) {
    let execution = {
        let executions = state.executions.read().await;
        executions.get(execution_id).cloned()
    };
    if let Some(execution) = execution {
        execution.cancel.cancel();
    }
}

async fn wait_for_project_executions_to_finish(
    state: &AppState,
    project_id: &str,
    runtime: &Arc<E2eQueueRuntime>,
) -> bool {
    loop {
        if runtime.cancel.is_cancelled() {
            return false;
        }

        let has_active_execution = {
            let executions = state.executions.read().await;
            executions
                .values()
                .any(|execution| execution.project_id == project_id)
        };

        if !has_active_execution {
            return true;
        }

        tokio::select! {
            _ = runtime.cancel.cancelled() => return false,
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }
}
