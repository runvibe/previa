use axum::extract::{Extension, Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Json, http::StatusCode};
use previa_runner::Pipeline;

use crate::server::auth::Principal;
use crate::server::db::{
    delete_pipeline_record, delete_pipeline_share_record, insert_project_pipeline_for_owner,
    load_pipeline_sharing_record, load_pipelines_for_project_accessible,
    load_project_pipeline_record, project_exists, update_pipeline_visibility_record,
    update_project_pipeline, upsert_pipeline_share_record,
};
use crate::server::errors::{
    bad_request_message_response, forbidden_response, internal_error_response, not_found_response,
};
use crate::server::execution::resolve_runtime_specs_for_execution;
use crate::server::models::{
    ErrorResponse, PipelineInput, PipelineShareCreateRequest, PipelineSharingRecord,
    PipelineVisibilityUpdateRequest, ProjectPipelineRecord,
};
use crate::server::services::pipeline_access::{PipelineAccess, can_access_pipeline};
use crate::server::services::pipeline_runtime::build_project_pipeline_record;
use crate::server::services::project_access::{ProjectAccess, can_access_project};
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
    Extension(principal): Extension<Principal>,
    Path(project_id): Path<String>,
) -> Response {
    match project_exists(&state.db, &project_id).await {
        Ok(false) => return not_found_response("project not found"),
        Ok(true) => {}
        Err(err) => return internal_error_response(format!("failed to load project: {err}")),
    }

    match load_pipelines_for_project_accessible(&state.db, &project_id, &principal).await {
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
    Extension(principal): Extension<Principal>,
    Path((project_id, pipeline_id)): Path<(String, String)>,
) -> Response {
    match project_exists(&state.db, &project_id).await {
        Ok(false) => return not_found_response("project not found"),
        Ok(true) => {}
        Err(err) => return internal_error_response(format!("failed to load project: {err}")),
    }

    match can_access_pipeline(
        &state.db,
        &project_id,
        &pipeline_id,
        &principal,
        PipelineAccess::Read,
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => return not_found_response("pipeline not found"),
        Err(err) => return internal_error_response(format!("failed to authorize pipeline: {err}")),
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
    Extension(principal): Extension<Principal>,
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

    match can_access_project(&state.db, &project_id, &principal, ProjectAccess::Write).await {
        Ok(true) => {}
        Ok(false) => return forbidden_response("project access denied"),
        Err(err) => return internal_error_response(format!("failed to authorize project: {err}")),
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

    match insert_project_pipeline_for_owner(
        &state.db,
        &project_id,
        pipeline,
        &principal.subject,
        &principal.username,
    )
    .await
    {
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
    Extension(principal): Extension<Principal>,
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

    match can_access_pipeline(
        &state.db,
        &project_id,
        &pipeline_id,
        &principal,
        PipelineAccess::Write,
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => return forbidden_response("pipeline access denied"),
        Err(err) => return internal_error_response(format!("failed to authorize pipeline: {err}")),
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
    Extension(principal): Extension<Principal>,
    Path((project_id, pipeline_id)): Path<(String, String)>,
) -> Response {
    match project_exists(&state.db, &project_id).await {
        Ok(false) => return not_found_response("project not found"),
        Ok(true) => {}
        Err(err) => return internal_error_response(format!("failed to load project: {err}")),
    }

    match can_access_pipeline(
        &state.db,
        &project_id,
        &pipeline_id,
        &principal,
        PipelineAccess::Delete,
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => {
            return match can_access_pipeline(
                &state.db,
                &project_id,
                &pipeline_id,
                &principal,
                PipelineAccess::Read,
            )
            .await
            {
                Ok(true) => forbidden_response("only the pipeline owner can delete it"),
                Ok(false) => not_found_response("pipeline not found"),
                Err(err) => internal_error_response(format!("failed to authorize pipeline: {err}")),
            };
        }
        Err(err) => return internal_error_response(format!("failed to authorize pipeline: {err}")),
    }

    match delete_pipeline_record(&state.db, &project_id, &pipeline_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => not_found_response("pipeline not found"),
        Err(err) => internal_error_response(format!("failed to delete pipeline: {err}")),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{projectId}/pipelines/{pipelineId}/shares",
    params(
        ("projectId" = String, Path, description = "ID do projeto"),
        ("pipelineId" = String, Path, description = "ID da pipeline")
    ),
    responses(
        (status = 200, description = "Compartilhamentos da pipeline", body = PipelineSharingRecord),
        (status = 404, description = "Pipeline não encontrada", body = ErrorResponse)
    )
)]
pub async fn get_project_pipeline_shares(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((project_id, pipeline_id)): Path<(String, String)>,
) -> Response {
    match can_access_pipeline(
        &state.db,
        &project_id,
        &pipeline_id,
        &principal,
        PipelineAccess::Read,
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => return not_found_response("pipeline not found"),
        Err(err) => return internal_error_response(format!("failed to authorize pipeline: {err}")),
    }

    match load_pipeline_sharing_record(&state.db, &project_id, &pipeline_id).await {
        Ok(Some(record)) => Json(record).into_response(),
        Ok(None) => not_found_response("pipeline not found"),
        Err(err) => internal_error_response(format!("failed to load pipeline shares: {err}")),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{projectId}/pipelines/{pipelineId}/shares",
    params(
        ("projectId" = String, Path, description = "ID do projeto"),
        ("pipelineId" = String, Path, description = "ID da pipeline")
    ),
    request_body = PipelineShareCreateRequest,
    responses(
        (status = 200, description = "Compartilhamento atualizado", body = PipelineSharingRecord),
        (status = 403, description = "Sem permissão para gerenciar compartilhamento", body = ErrorResponse),
        (status = 404, description = "Pipeline não encontrada", body = ErrorResponse)
    )
)]
pub async fn upsert_project_pipeline_share(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((project_id, pipeline_id)): Path<(String, String)>,
    Json(payload): Json<PipelineShareCreateRequest>,
) -> Response {
    if payload.user_id.trim().is_empty() || payload.username.trim().is_empty() {
        return bad_request_message_response("userId and username are required");
    }
    match can_access_pipeline(
        &state.db,
        &project_id,
        &pipeline_id,
        &principal,
        PipelineAccess::Manage,
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => return forbidden_response("only the pipeline owner can share it"),
        Err(err) => return internal_error_response(format!("failed to authorize pipeline: {err}")),
    }

    if let Err(err) = upsert_pipeline_share_record(
        &state.db,
        &pipeline_id,
        payload.user_id.trim(),
        payload.username.trim(),
        payload.access_level,
    )
    .await
    {
        return internal_error_response(format!("failed to share pipeline: {err}"));
    }

    match load_pipeline_sharing_record(&state.db, &project_id, &pipeline_id).await {
        Ok(Some(record)) => Json(record).into_response(),
        Ok(None) => not_found_response("pipeline not found"),
        Err(err) => internal_error_response(format!("failed to load pipeline shares: {err}")),
    }
}

#[utoipa::path(
    delete,
    path = "/api/v1/projects/{projectId}/pipelines/{pipelineId}/shares/{userId}",
    params(
        ("projectId" = String, Path, description = "ID do projeto"),
        ("pipelineId" = String, Path, description = "ID da pipeline"),
        ("userId" = String, Path, description = "ID do usuário")
    ),
    responses(
        (status = 204, description = "Compartilhamento removido"),
        (status = 403, description = "Sem permissão para revogar compartilhamento", body = ErrorResponse),
        (status = 404, description = "Pipeline ou compartilhamento não encontrado", body = ErrorResponse)
    )
)]
pub async fn delete_project_pipeline_share(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((project_id, pipeline_id, user_id)): Path<(String, String, String)>,
) -> Response {
    match can_access_pipeline(
        &state.db,
        &project_id,
        &pipeline_id,
        &principal,
        PipelineAccess::Manage,
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => return forbidden_response("only the pipeline owner can revoke access"),
        Err(err) => return internal_error_response(format!("failed to authorize pipeline: {err}")),
    }

    match delete_pipeline_share_record(&state.db, &pipeline_id, &user_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => not_found_response("pipeline share not found"),
        Err(err) => internal_error_response(format!("failed to revoke pipeline share: {err}")),
    }
}

#[utoipa::path(
    put,
    path = "/api/v1/projects/{projectId}/pipelines/{pipelineId}/visibility",
    params(
        ("projectId" = String, Path, description = "ID do projeto"),
        ("pipelineId" = String, Path, description = "ID da pipeline")
    ),
    request_body = PipelineVisibilityUpdateRequest,
    responses(
        (status = 200, description = "Visibilidade atualizada", body = PipelineSharingRecord),
        (status = 403, description = "Sem permissão para alterar visibilidade", body = ErrorResponse),
        (status = 404, description = "Pipeline não encontrada", body = ErrorResponse)
    )
)]
pub async fn update_project_pipeline_visibility(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((project_id, pipeline_id)): Path<(String, String)>,
    Json(payload): Json<PipelineVisibilityUpdateRequest>,
) -> Response {
    match can_access_pipeline(
        &state.db,
        &project_id,
        &pipeline_id,
        &principal,
        PipelineAccess::Manage,
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => return forbidden_response("only the pipeline owner can change visibility"),
        Err(err) => return internal_error_response(format!("failed to authorize pipeline: {err}")),
    }

    match update_pipeline_visibility_record(
        &state.db,
        &project_id,
        &pipeline_id,
        payload.visibility,
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => return not_found_response("pipeline not found"),
        Err(err) => return internal_error_response(format!("failed to update visibility: {err}")),
    }

    match load_pipeline_sharing_record(&state.db, &project_id, &pipeline_id).await {
        Ok(Some(record)) => Json(record).into_response(),
        Ok(None) => not_found_response("pipeline not found"),
        Err(err) => internal_error_response(format!("failed to load pipeline shares: {err}")),
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

    use crate::server::auth::AuthRuntime;
    use crate::server::auth::config::AuthConfig;
    use crate::server::auth::permissions::Role;
    use crate::server::build_app;
    use crate::server::db::{insert_project_pipeline, insert_project_pipeline_for_owner};
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

    #[tokio::test]
    async fn protected_public_pipeline_allows_anonymous_update() {
        let state = protected_test_state().await;
        seed_project_with_pipeline(&state, "project-1", pipeline("pipe-1")).await;
        mark_pipeline_public(&state, "pipe-1", "usr_owner", "owner").await;
        let app = app_with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri("/api/v1/projects/project-1/pipelines/pipe-1")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "name": "anonymous_edit",
                            "description": "edited without a bearer token",
                            "steps": []
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn protected_public_pipeline_rejects_anonymous_delete_when_owner_is_named_user() {
        let state = protected_test_state().await;
        seed_project_with_pipeline(&state, "project-1", pipeline("pipe-1")).await;
        mark_pipeline_public(&state, "pipe-1", "usr_owner", "owner").await;
        let app = app_with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri("/api/v1/projects/project-1/pipelines/pipe-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn protected_public_pipeline_allows_anonymous_delete_when_owner_is_anonymous() {
        let state = protected_test_state().await;
        seed_project_with_pipeline(&state, "project-1", pipeline("pipe-1")).await;
        mark_pipeline_public(&state, "pipe-1", "anonymous", "anonymous").await;
        let app = app_with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri("/api/v1/projects/project-1/pipelines/pipe-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn owner_can_share_private_pipeline_and_revoke_shared_editor() {
        let state = protected_test_state().await;
        seed_project_with_pipeline_for_owner(
            &state,
            "project-1",
            pipeline("pipe-1"),
            "usr_owner",
            "owner",
        )
        .await;
        let owner_token = jwt(&state.auth, "usr_owner", "owner", Role::Editor);
        let shared_token = jwt(&state.auth, "usr_shared", "shared", Role::Editor);
        let app = app_with_state(state);

        let share = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/projects/project-1/pipelines/pipe-1/shares")
                    .header("authorization", format!("Bearer {owner_token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"userId":"usr_shared","username":"shared","accessLevel":"editor"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(share.status(), StatusCode::OK);

        let shared_update = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri("/api/v1/projects/project-1/pipelines/pipe-1")
                    .header("authorization", format!("Bearer {shared_token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "name": "shared_edit",
                            "description": "edited by shared user",
                            "steps": []
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(shared_update.status(), StatusCode::OK);

        let revoke = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri("/api/v1/projects/project-1/pipelines/pipe-1/shares/usr_shared")
                    .header("authorization", format!("Bearer {owner_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revoke.status(), StatusCode::NO_CONTENT);

        let blocked_update = app
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri("/api/v1/projects/project-1/pipelines/pipe-1")
                    .header("authorization", format!("Bearer {shared_token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "name": "blocked_edit",
                            "description": null,
                            "steps": []
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(blocked_update.status(), StatusCode::FORBIDDEN);
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
        test_state_with_auth(crate::server::auth::AuthRuntime::anonymous()).await
    }

    async fn protected_test_state() -> AppState {
        test_state_with_auth(protected_auth()).await
    }

    fn protected_auth() -> AuthRuntime {
        let config = AuthConfig::from_env_values(&[
            ("PREVIA_AUTH_ANONYMOUS", "false"),
            ("PREVIA_ROOT_USERNAME", "root"),
            ("PREVIA_ROOT_PASSWORD", "secret"),
            ("PREVIA_JWT_SECRET", "test-jwt-secret"),
        ])
        .expect("protected auth config");
        AuthRuntime::from_config(config).expect("protected auth runtime")
    }

    fn jwt(auth: &AuthRuntime, subject: &str, username: &str, role: Role) -> String {
        auth.jwt
            .as_ref()
            .expect("jwt")
            .issue(subject, username, role, "database")
            .expect("issue jwt")
    }

    async fn test_state_with_auth(auth: AuthRuntime) -> AppState {
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
            auth,
            rps_per_node: 1,
            scheduler: ExecutionScheduler::new(Default::default()),
            executions: Arc::new(RwLock::new(HashMap::new())),
            e2e_queues: Arc::new(RwLock::new(HashMap::new())),
            mcp_sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn mark_pipeline_public(
        state: &AppState,
        pipeline_id: &str,
        owner_user_id: &str,
        owner_username: &str,
    ) {
        sqlx::query(
            "UPDATE pipelines
            SET visibility = 'public', owner_user_id = ?, owner_username = ?
            WHERE id = ?",
        )
        .bind(owner_user_id)
        .bind(owner_username)
        .bind(pipeline_id)
        .execute(&state.db)
        .await
        .expect("mark pipeline public");
    }

    async fn seed_project_with_pipeline(state: &AppState, project_id: &str, pipeline: Pipeline) {
        insert_project_pipeline(&state.db, project_id, pipeline)
            .await
            .expect("insert pipeline");
    }

    async fn seed_project_with_pipeline_for_owner(
        state: &AppState,
        project_id: &str,
        pipeline: Pipeline,
        owner_user_id: &str,
        owner_username: &str,
    ) {
        insert_project_pipeline_for_owner(
            &state.db,
            project_id,
            pipeline,
            owner_user_id,
            owner_username,
        )
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
