use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::server::auth::Principal;
use crate::server::db::{
    list_project_records, load_pipelines_for_project_accessible, load_project_record,
    upsert_project_metadata, upsert_project_with_pipelines_for_owner,
};
use crate::server::errors::{internal_error_response, not_found_response};
use crate::server::models::{
    ErrorResponse, ProjectListQuery, ProjectMetadataUpsertRequest, ProjectRecord,
    ProjectUpsertRequest,
};
use crate::server::state::AppState;
use crate::server::utils::new_uuid_v7;

#[utoipa::path(
    get,
    path = "/api/v1/projects",
    params(
        ("limit" = Option<u32>, Query, description = "Limite de registros retornados (default 100, max 500)"),
        ("offset" = Option<u32>, Query, description = "Deslocamento da paginação (default 0)"),
        ("order" = Option<crate::server::models::HistoryOrder>, Query, description = "Ordem por atualização: asc | desc (default desc)")
    ),
    responses(
        (
            status = 200,
            description = "Lista de projetos",
            body = Vec<ProjectRecord>
        ),
        (
            status = 500,
            description = "Erro ao consultar projetos",
            body = ErrorResponse
        )
    )
)]
pub async fn list_projects(
    State(state): State<AppState>,
    Query(query): Query<ProjectListQuery>,
) -> Response {
    match list_project_records(&state.db, query).await {
        Ok(items) => Json(items).into_response(),
        Err(err) => internal_error_response(format!("failed to query projects: {err}")),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{projectId}",
    params(
        ("projectId" = String, Path, description = "ID do projeto")
    ),
    responses(
        (
            status = 200,
            description = "Projeto",
            body = ProjectRecord
        ),
        (
            status = 404,
            description = "Projeto não encontrado",
            body = ErrorResponse
        )
    )
)]
pub async fn get_project(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(project_id): Path<String>,
) -> Response {
    match load_project_record(&state.db, &project_id).await {
        Ok(Some(project)) => {
            match load_pipelines_for_project_accessible(&state.db, &project_id, &principal).await {
                Ok(pipelines)
                    if !pipelines.is_empty()
                        || matches!(
                            principal.role,
                            crate::server::auth::permissions::Role::Root
                                | crate::server::auth::permissions::Role::Admin
                                | crate::server::auth::permissions::Role::Editor
                                | crate::server::auth::permissions::Role::Operator
                                | crate::server::auth::permissions::Role::Viewer
                        ) =>
                {
                    Json(project).into_response()
                }
                Ok(_) => not_found_response("project not found"),
                Err(err) => internal_error_response(format!("failed to authorize project: {err}")),
            }
        }
        Ok(None) => not_found_response("project not found"),
        Err(err) => internal_error_response(format!("failed to load project: {err}")),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/projects",
    request_body = ProjectUpsertRequest,
    responses(
        (
            status = 201,
            description = "Projeto criado",
            body = ProjectRecord
        ),
        (
            status = 400,
            description = "Payload inválido",
            body = ErrorResponse
        )
    )
)]
pub async fn create_project(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(payload): Json<ProjectUpsertRequest>,
) -> Response {
    if payload.name.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "bad_request".to_owned(),
                message: "project name is required".to_owned(),
            }),
        )
            .into_response();
    }

    let project_id = new_uuid_v7();
    match upsert_project_with_pipelines_for_owner(
        &state.db,
        project_id,
        payload,
        &principal.subject,
        &principal.username,
    )
    .await
    {
        Ok(project) => (StatusCode::CREATED, Json(project)).into_response(),
        Err(err) => internal_error_response(format!("failed to create project: {err}")),
    }
}

#[utoipa::path(
    put,
    path = "/api/v1/projects/{projectId}",
    params(
        ("projectId" = String, Path, description = "ID do projeto")
    ),
    request_body = ProjectMetadataUpsertRequest,
    responses(
        (
            status = 200,
            description = "Projeto atualizado",
            body = ProjectRecord
        ),
        (
            status = 400,
            description = "Payload inválido",
            body = ErrorResponse
        )
    )
)]
pub async fn upsert_project(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    Json(payload): Json<ProjectMetadataUpsertRequest>,
) -> Response {
    if payload.name.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "bad_request".to_owned(),
                message: "project name is required".to_owned(),
            }),
        )
            .into_response();
    }

    match upsert_project_metadata(&state.db, project_id, payload).await {
        Ok(project) => Json(project).into_response(),
        Err(err) => internal_error_response(format!("failed to upsert project: {err}")),
    }
}

#[utoipa::path(
    delete,
    path = "/api/v1/projects/{projectId}",
    params(
        ("projectId" = String, Path, description = "ID do projeto")
    ),
    responses(
        (status = 204, description = "Projeto removido"),
        (status = 404, description = "Projeto não encontrado", body = ErrorResponse)
    )
)]
pub async fn delete_project(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
) -> Response {
    match sqlx::query("DELETE FROM projects WHERE id = ?")
        .bind(&project_id)
        .execute(&state.db)
        .await
    {
        Ok(result) if result.rows_affected() > 0 => StatusCode::NO_CONTENT.into_response(),
        Ok(_) => not_found_response("project not found"),
        Err(err) => internal_error_response(format!("failed to delete project: {err}")),
    }
}
