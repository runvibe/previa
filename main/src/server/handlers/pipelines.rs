use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Json, http::StatusCode};
use previa_runner::Pipeline;

use crate::server::db::{
    delete_pipeline_record, insert_project_pipeline, load_pipelines_for_project,
    load_project_pipeline_record, project_exists, update_project_pipeline,
};
use crate::server::errors::{
    bad_request_message_response, internal_error_response, not_found_response,
};
use crate::server::execution::resolve_runtime_specs_for_execution;
use crate::server::models::{ErrorResponse, PipelineInput, ProjectPipelineRecord};
use crate::server::services::pipeline_runtime::build_project_pipeline_record;
use crate::server::state::AppState;
use crate::server::utils::new_uuid_v7;
use crate::server::validation::pipelines::validate_pipeline_templates;

#[utoipa::path(
    get,
    path = "/api/v1/projects/{projectId}/pipelines",
    params(
        ("projectId" = String, Path, description = "ID do projeto")
    ),
    responses(
        (
            status = 200,
            description = "Lista de pipelines do projeto",
            body = Vec<Pipeline>
        ),
        (
            status = 404,
            description = "Projeto não encontrado",
            body = ErrorResponse
        )
    )
)]
pub async fn list_project_pipelines(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
) -> Response {
    match project_exists(&state.db, &project_id).await {
        Ok(false) => return not_found_response("project not found"),
        Ok(true) => {}
        Err(err) => return internal_error_response(format!("failed to load project: {err}")),
    }

    match load_pipelines_for_project(&state.db, &project_id).await {
        Ok(pipelines) => Json(pipelines).into_response(),
        Err(err) => internal_error_response(format!("failed to load project pipelines: {err}")),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{projectId}/pipelines/{pipelineId}",
    params(
        ("projectId" = String, Path, description = "ID do projeto"),
        ("pipelineId" = String, Path, description = "ID da pipeline")
    ),
    responses(
        (
            status = 200,
            description = "Pipeline do projeto",
            body = ProjectPipelineRecord
        ),
        (
            status = 404,
            description = "Projeto ou pipeline não encontrado",
            body = ErrorResponse
        )
    )
)]
pub async fn get_project_pipeline(
    State(state): State<AppState>,
    Path((project_id, pipeline_id)): Path<(String, String)>,
) -> Response {
    match project_exists(&state.db, &project_id).await {
        Ok(false) => return not_found_response("project not found"),
        Ok(true) => {}
        Err(err) => return internal_error_response(format!("failed to load project: {err}")),
    }

    match load_project_pipeline_record(&state.db, &project_id, &pipeline_id).await {
        Ok(Some(pipeline)) => {
            Json(build_project_pipeline_record(&state, &project_id, pipeline).await).into_response()
        }
        Ok(None) => not_found_response("pipeline not found"),
        Err(err) => internal_error_response(format!("failed to load pipeline: {err}")),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{projectId}/pipelines",
    params(
        ("projectId" = String, Path, description = "ID do projeto")
    ),
    request_body = PipelineInput,
    responses(
        (
            status = 201,
            description = "Pipeline criada",
            body = Pipeline
        ),
        (
            status = 400,
            description = "Payload inválido",
            body = ErrorResponse
        ),
        (
            status = 404,
            description = "Projeto não encontrado",
            body = ErrorResponse
        )
    )
)]
pub async fn create_project_pipeline(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    Json(pipeline): Json<PipelineInput>,
) -> Response {
    if pipeline.name.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "bad_request".to_owned(),
                message: "pipeline name is required".to_owned(),
            }),
        )
            .into_response();
    }

    match project_exists(&state.db, &project_id).await {
        Ok(false) => return not_found_response("project not found"),
        Ok(true) => {}
        Err(err) => return internal_error_response(format!("failed to load project: {err}")),
    }

    let pipeline = Pipeline {
        id: Some(new_uuid_v7()),
        name: pipeline.name,
        description: pipeline.description,
        steps: pipeline.steps,
    };
    let runtime_specs =
        match resolve_runtime_specs_for_execution(&state.db, Some(&project_id), &[]).await {
            Ok(specs) => specs,
            Err(err) => {
                return internal_error_response(format!(
                    "failed to load project specs for pipeline validation: {err}"
                ));
            }
        };
    let template_errors =
        validate_pipeline_templates(&pipeline, runtime_specs.as_deref(), None, None);
    if !template_errors.is_empty() {
        return bad_request_message_response(&template_errors.join("; "));
    }

    match insert_project_pipeline(&state.db, &project_id, pipeline).await {
        Ok(item) => (StatusCode::CREATED, Json(item)).into_response(),
        Err(err) => internal_error_response(format!("failed to create pipeline: {err}")),
    }
}

#[utoipa::path(
    put,
    path = "/api/v1/projects/{projectId}/pipelines/{pipelineId}",
    params(
        ("projectId" = String, Path, description = "ID do projeto"),
        ("pipelineId" = String, Path, description = "ID da pipeline")
    ),
    request_body = PipelineInput,
    responses(
        (
            status = 200,
            description = "Pipeline atualizada",
            body = Pipeline
        ),
        (
            status = 400,
            description = "Payload inválido",
            body = ErrorResponse
        ),
        (
            status = 404,
            description = "Projeto ou pipeline não encontrado",
            body = ErrorResponse
        )
    )
)]
pub async fn upsert_project_pipeline(
    State(state): State<AppState>,
    Path((project_id, pipeline_id)): Path<(String, String)>,
    Json(pipeline): Json<PipelineInput>,
) -> Response {
    if pipeline.name.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "bad_request".to_owned(),
                message: "pipeline name is required".to_owned(),
            }),
        )
            .into_response();
    }

    match project_exists(&state.db, &project_id).await {
        Ok(false) => return not_found_response("project not found"),
        Ok(true) => {}
        Err(err) => return internal_error_response(format!("failed to load project: {err}")),
    }

    let pipeline = Pipeline {
        id: Some(pipeline_id.clone()),
        name: pipeline.name,
        description: pipeline.description,
        steps: pipeline.steps,
    };
    let runtime_specs =
        match resolve_runtime_specs_for_execution(&state.db, Some(&project_id), &[]).await {
            Ok(specs) => specs,
            Err(err) => {
                return internal_error_response(format!(
                    "failed to load project specs for pipeline validation: {err}"
                ));
            }
        };
    let template_errors =
        validate_pipeline_templates(&pipeline, runtime_specs.as_deref(), None, None);
    if !template_errors.is_empty() {
        return bad_request_message_response(&template_errors.join("; "));
    }
    match update_project_pipeline(&state.db, &project_id, &pipeline_id, pipeline).await {
        Ok(Some(item)) => Json(item).into_response(),
        Ok(None) => not_found_response("pipeline not found"),
        Err(err) => internal_error_response(format!("failed to update pipeline: {err}")),
    }
}

#[utoipa::path(
    delete,
    path = "/api/v1/projects/{projectId}/pipelines/{pipelineId}",
    params(
        ("projectId" = String, Path, description = "ID do projeto"),
        ("pipelineId" = String, Path, description = "ID da pipeline")
    ),
    responses(
        (status = 204, description = "Pipeline removida"),
        (status = 404, description = "Projeto ou pipeline não encontrado", body = ErrorResponse)
    )
)]
pub async fn delete_project_pipeline(
    State(state): State<AppState>,
    Path((project_id, pipeline_id)): Path<(String, String)>,
) -> Response {
    match project_exists(&state.db, &project_id).await {
        Ok(false) => return not_found_response("project not found"),
        Ok(true) => {}
        Err(err) => return internal_error_response(format!("failed to load project: {err}")),
    }

    match delete_pipeline_record(&state.db, &project_id, &pipeline_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => not_found_response("pipeline not found"),
        Err(err) => internal_error_response(format!("failed to delete pipeline: {err}")),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request, StatusCode};
    use previa_runner::{Pipeline, PipelineStep};
    use serde_json::{Value, json};
    use tokio::sync::{RwLock, broadcast};
    use tokio_util::sync::CancellationToken;
    use tower::ServiceExt;

    use crate::server::build_app;
    use crate::server::db::insert_project_pipeline;
    use crate::server::execution::ExecutionScheduler;
    use crate::server::execution::scheduler::SharedValue;
    use crate::server::mcp::models::McpConfig;
    use crate::server::models::{E2eQueuePipelineRecord, E2eQueueRecord, E2eQueueStatus};
    use crate::server::state::{AppState, E2eQueueRuntime, ExecutionCtx, ExecutionKind};

    #[tokio::test]
    async fn get_project_pipeline_returns_running_runtime_for_active_execution() {
        let state = test_state().await;
        seed_project_with_pipeline(&state, "project-1", pipeline("pipe-1")).await;
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
        let app = app_with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/projects/project-1/pipelines/pipe-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let payload = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(payload["runtime"]["status"], json!("running"));
        assert_eq!(payload["runtime"]["activeExecution"]["id"], json!("exec-1"));
        assert_eq!(payload["runtime"]["activeExecution"]["kind"], json!("load"));
    }

    #[tokio::test]
    async fn get_project_pipeline_returns_queued_runtime_for_pending_queue_item() {
        let state = test_state().await;
        seed_project_with_pipeline(&state, "project-1", pipeline("pipe-1")).await;
        let queue = E2eQueueRuntime::new(
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
            queues.insert("project-1".to_owned(), queue);
        }
        let app = app_with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/projects/project-1/pipelines/pipe-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let payload = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(payload["runtime"]["status"], json!("queued"));
        assert_eq!(payload["runtime"]["activeQueue"]["id"], json!("queue-1"));
        assert!(payload["runtime"]["activeExecution"].is_null());
    }

    fn app_with_state(state: AppState) -> Router {
        build_app(
            state,
            &McpConfig {
                enabled: false,
                path: "/mcp".to_owned(),
            },
        )
    }

    async fn test_state() -> AppState {
        let db = crate::server::db::DbPool::connect("sqlite::memory:", 1)
            .await
            .expect("sqlite memory db");
        sqlx::migrate!("./migrations/sqlite")
            .run(db.pool())
            .await
            .expect("migrations");
        sqlx::query(
            "INSERT INTO projects (
                id, name, description, created_at, updated_at, created_at_ms, updated_at_ms, spec_json, execution_backend_url
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("project-1")
        .bind("Project")
        .bind(Option::<String>::None)
        .bind("2026-03-13T00:00:00.000Z")
        .bind("2026-03-13T00:00:00.000Z")
        .bind(0_i64)
        .bind(0_i64)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .execute(&db)
        .await
        .expect("insert project");

        AppState {
            client: reqwest::Client::new(),
            db,
            context_name: "default".to_owned(),
            runner_auth_key: None,
            auth: crate::server::auth::AuthRuntime::anonymous(),
            rps_per_node: 1,
            scheduler: ExecutionScheduler::new(Default::default()),
            executions: Arc::new(RwLock::new(HashMap::new())),
            e2e_queues: Arc::new(RwLock::new(HashMap::new())),
            mcp_sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn seed_project_with_pipeline(state: &AppState, project_id: &str, pipeline: Pipeline) {
        insert_project_pipeline(&state.db, project_id, pipeline)
            .await
            .expect("insert pipeline");
    }

    fn pipeline(id: &str) -> Pipeline {
        Pipeline {
            id: Some(id.to_owned()),
            name: format!("Pipeline {id}"),
            description: None,
            steps: vec![PipelineStep {
                id: "step-1".to_owned(),
                name: "step-1".to_owned(),
                description: None,
                method: "GET".to_owned(),
                url: "https://example.com".to_owned(),
                headers: HashMap::new(),
                body: None,
                operation_id: None,
                delay: None,
                retry: None,
                asserts: Vec::new(),
            }],
        }
    }
}
