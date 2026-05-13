use previa_runner::Pipeline;

use crate::server::models::{
    E2eQueueStatus, PipelineExecutionKind, PipelineExecutionRef, PipelineQueueRef,
    PipelineRuntimeState, PipelineRuntimeStatus, ProjectPipelineRecord,
};
use crate::server::state::{AppState, ExecutionKind};

pub async fn build_project_pipeline_record(
    state: &AppState,
    project_id: &str,
    pipeline: Pipeline,
) -> ProjectPipelineRecord {
    let runtime = match pipeline.id.as_deref() {
        Some(pipeline_id) => pipeline_runtime(state, project_id, pipeline_id).await,
        None => PipelineRuntimeState::idle(),
    };

    ProjectPipelineRecord {
        id: pipeline.id,
        name: pipeline.name,
        description: pipeline.description,
        steps: pipeline.steps,
        runtime,
    }
}

pub async fn pipeline_runtime(
    state: &AppState,
    project_id: &str,
    pipeline_id: &str,
) -> PipelineRuntimeState {
    if let Some(runtime) = active_execution_runtime(state, project_id, pipeline_id).await {
        return runtime;
    }

    if let Some(runtime) = active_queue_runtime(state, project_id, pipeline_id).await {
        return runtime;
    }

    PipelineRuntimeState::idle()
}

async fn active_execution_runtime(
    state: &AppState,
    project_id: &str,
    pipeline_id: &str,
) -> Option<PipelineRuntimeState> {
    let executions = {
        let executions = state.executions.read().await;
        executions
            .iter()
            .filter(|(_, execution)| {
                execution.project_id == project_id
                    && execution.pipeline_id.as_deref() == Some(pipeline_id)
            })
            .map(|(execution_id, execution)| {
                (
                    execution_id.clone(),
                    execution.kind,
                    execution.init_payload.clone(),
                )
            })
            .collect::<Vec<_>>()
    };

    let mut queued = None;
    for (execution_id, kind, init_payload) in executions {
        let payload = init_payload.get().await;
        let Some(status) = payload.get("status").and_then(|status| status.as_str()) else {
            continue;
        };

        let execution_ref = PipelineExecutionRef {
            id: execution_id,
            kind: match kind {
                ExecutionKind::E2e => PipelineExecutionKind::E2e,
                ExecutionKind::Load => PipelineExecutionKind::Load,
            },
        };
        match status {
            "running" => {
                return Some(PipelineRuntimeState {
                    status: PipelineRuntimeStatus::Running,
                    active_execution: Some(execution_ref),
                    active_queue: None,
                });
            }
            "queued" => {
                queued = Some(PipelineRuntimeState {
                    status: PipelineRuntimeStatus::Queued,
                    active_execution: Some(execution_ref),
                    active_queue: None,
                });
            }
            _ => {}
        }
    }

    queued
}

async fn active_queue_runtime(
    state: &AppState,
    project_id: &str,
    pipeline_id: &str,
) -> Option<PipelineRuntimeState> {
    let runtime = {
        let queues = state.e2e_queues.read().await;
        queues.get(project_id).cloned()
    }?;

    let snapshot = runtime.snapshot().await;
    let pipeline = snapshot
        .pipelines
        .iter()
        .find(|pipeline| {
            pipeline.id == pipeline_id
                && matches!(
                    pipeline.status,
                    E2eQueueStatus::Pending | E2eQueueStatus::Running
                )
        })?
        .clone();

    let queue_ref = Some(PipelineQueueRef { id: snapshot.id });
    match pipeline.status {
        E2eQueueStatus::Running => Some(PipelineRuntimeState {
            status: PipelineRuntimeStatus::Running,
            active_execution: runtime
                .active_execution_id()
                .await
                .map(|id| PipelineExecutionRef {
                    id,
                    kind: PipelineExecutionKind::E2e,
                }),
            active_queue: queue_ref,
        }),
        E2eQueueStatus::Pending => Some(PipelineRuntimeState {
            status: PipelineRuntimeStatus::Queued,
            active_execution: None,
            active_queue: queue_ref,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use serde_json::json;
    use tokio::sync::{RwLock, broadcast};
    use tokio_util::sync::CancellationToken;

    use super::pipeline_runtime;
    use crate::server::execution::ExecutionScheduler;
    use crate::server::execution::scheduler::SharedValue;
    use crate::server::models::{
        E2eQueuePipelineRecord, E2eQueueRecord, E2eQueueStatus, PipelineRuntimeStatus,
    };
    use crate::server::state::{AppState, E2eQueueRuntime, ExecutionCtx, ExecutionKind};

    #[tokio::test]
    async fn returns_running_runtime_for_active_execution() {
        let state = empty_state().await;
        {
            let mut executions = state.executions.write().await;
            let (sse_tx, _) = broadcast::channel(8);
            executions.insert(
                "exec-1".to_owned(),
                Arc::new(ExecutionCtx {
                    cancel: CancellationToken::new(),
                    project_id: "project-1".to_owned(),
                    pipeline_id: Some("pipe-1".to_owned()),
                    kind: ExecutionKind::Load,
                    sse_tx,
                    init_payload: SharedValue::new(json!({ "status": "running" })),
                    snapshot_payload: SharedValue::new(json!({
                        "executionId": "exec-1",
                        "status": "running",
                        "kind": "load",
                        "context": {},
                        "lines": [],
                        "consolidated": null,
                        "errors": []
                    })),
                }),
            );
        }

        let runtime = pipeline_runtime(&state, "project-1", "pipe-1").await;
        assert!(matches!(runtime.status, PipelineRuntimeStatus::Running));
        assert_eq!(
            runtime
                .active_execution
                .as_ref()
                .map(|execution| execution.id.as_str()),
            Some("exec-1")
        );
        assert!(runtime.active_queue.is_none());
    }

    #[tokio::test]
    async fn returns_queued_runtime_for_pending_queue_item() {
        let state = empty_state().await;
        let runtime = E2eQueueRuntime::new(
            "queue-1".to_owned(),
            "project-1".to_owned(),
            E2eQueueRecord {
                id: "queue-1".to_owned(),
                status: E2eQueueStatus::Pending,
                pipelines: vec![E2eQueuePipelineRecord {
                    id: "pipe-1".to_owned(),
                    status: E2eQueueStatus::Pending,
                    updated_at: "2026-03-13T00:00:00.000Z".to_owned(),
                }],
                updated_at: "2026-03-13T00:00:00.000Z".to_owned(),
            },
        );
        {
            let mut queues = state.e2e_queues.write().await;
            queues.insert("project-1".to_owned(), runtime);
        }

        let runtime = pipeline_runtime(&state, "project-1", "pipe-1").await;
        assert!(matches!(runtime.status, PipelineRuntimeStatus::Queued));
        assert_eq!(
            runtime.active_queue.as_ref().map(|queue| queue.id.as_str()),
            Some("queue-1")
        );
        assert!(runtime.active_execution.is_none());
    }

    async fn empty_state() -> AppState {
        let db = crate::server::db::DbPool::connect("sqlite::memory:", 1)
            .await
            .expect("sqlite memory db");
        sqlx::migrate!("./migrations/sqlite")
            .run(db.pool())
            .await
            .expect("migrations");

        AppState {
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
        }
    }
}
