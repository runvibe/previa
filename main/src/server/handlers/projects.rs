use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::server::auth::Principal;
use crate::server::db::{
    delete_project_share_record, list_project_records_accessible,
    load_pipelines_for_project_accessible, load_project_record, load_project_sharing_record,
    update_project_visibility_record, upsert_project_metadata, upsert_project_share_record,
    upsert_project_with_pipelines_for_owner,
};
use crate::server::errors::{
    bad_request_message_response, forbidden_response, internal_error_response, not_found_response,
};
use crate::server::models::{
    ErrorResponse, ProjectListQuery, ProjectMetadataUpsertRequest, ProjectRecord,
    ProjectShareCreateRequest, ProjectSharingRecord, ProjectUpsertRequest,
    ProjectVisibilityUpdateRequest,
};
use crate::server::services::project_access::{ProjectAccess, can_access_project};
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
    Extension(principal): Extension<Principal>,
    Query(query): Query<ProjectListQuery>,
) -> Response {
    match list_project_records_accessible(&state.db, query, &principal).await {
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
            match can_access_project(&state.db, &project_id, &principal, ProjectAccess::Read).await
            {
                Ok(true) => Json(project).into_response(),
                Ok(false) => {
                    match load_pipelines_for_project_accessible(&state.db, &project_id, &principal)
                        .await
                    {
                        Ok(pipelines) if !pipelines.is_empty() => Json(project).into_response(),
                        Ok(_) => not_found_response("project not found"),
                        Err(err) => {
                            internal_error_response(format!("failed to authorize project: {err}"))
                        }
                    }
                }
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
    Extension(principal): Extension<Principal>,
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

    match can_access_project(&state.db, &project_id, &principal, ProjectAccess::Write).await {
        Ok(true) => {}
        Ok(false) => return forbidden_response("project access denied"),
        Err(err) => return internal_error_response(format!("failed to authorize project: {err}")),
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
    Extension(principal): Extension<Principal>,
    Path(project_id): Path<String>,
) -> Response {
    match can_access_project(&state.db, &project_id, &principal, ProjectAccess::Delete).await {
        Ok(true) => {}
        Ok(false) => {
            return match can_access_project(&state.db, &project_id, &principal, ProjectAccess::Read)
                .await
            {
                Ok(true) => forbidden_response("only the stack owner can delete it"),
                Ok(false) => not_found_response("project not found"),
                Err(err) => internal_error_response(format!("failed to authorize project: {err}")),
            };
        }
        Err(err) => return internal_error_response(format!("failed to authorize project: {err}")),
    }

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

#[utoipa::path(
    get,
    path = "/api/v1/projects/{projectId}/shares",
    params(("projectId" = String, Path, description = "ID do projeto")),
    responses(
        (status = 200, description = "Compartilhamentos da stack", body = ProjectSharingRecord),
        (status = 404, description = "Projeto não encontrado", body = ErrorResponse)
    )
)]
pub async fn get_project_shares(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(project_id): Path<String>,
) -> Response {
    match can_access_project(&state.db, &project_id, &principal, ProjectAccess::Read).await {
        Ok(true) => {}
        Ok(false) => {
            match load_pipelines_for_project_accessible(&state.db, &project_id, &principal).await {
                Ok(pipelines) if !pipelines.is_empty() => {}
                Ok(_) => return not_found_response("project not found"),
                Err(err) => {
                    return internal_error_response(format!("failed to authorize project: {err}"));
                }
            };
        }
        Err(err) => return internal_error_response(format!("failed to authorize project: {err}")),
    }

    match load_project_sharing_record(&state.db, &project_id).await {
        Ok(Some(record)) => Json(record).into_response(),
        Ok(None) => not_found_response("project not found"),
        Err(err) => internal_error_response(format!("failed to load project shares: {err}")),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{projectId}/shares",
    params(("projectId" = String, Path, description = "ID do projeto")),
    request_body = ProjectShareCreateRequest,
    responses(
        (status = 200, description = "Compartilhamento atualizado", body = ProjectSharingRecord),
        (status = 403, description = "Sem permissão para compartilhar stack", body = ErrorResponse),
        (status = 404, description = "Projeto não encontrado", body = ErrorResponse)
    )
)]
pub async fn upsert_project_share(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(project_id): Path<String>,
    Json(payload): Json<ProjectShareCreateRequest>,
) -> Response {
    if payload.user_id.trim().is_empty() || payload.username.trim().is_empty() {
        return bad_request_message_response("userId and username are required");
    }
    match can_access_project(&state.db, &project_id, &principal, ProjectAccess::Manage).await {
        Ok(true) => {}
        Ok(false) => return forbidden_response("only the stack owner can share it"),
        Err(err) => return internal_error_response(format!("failed to authorize project: {err}")),
    }

    if let Err(err) = upsert_project_share_record(
        &state.db,
        &project_id,
        payload.user_id.trim(),
        payload.username.trim(),
        payload.access_level,
    )
    .await
    {
        return internal_error_response(format!("failed to share project: {err}"));
    }

    match load_project_sharing_record(&state.db, &project_id).await {
        Ok(Some(record)) => Json(record).into_response(),
        Ok(None) => not_found_response("project not found"),
        Err(err) => internal_error_response(format!("failed to load project shares: {err}")),
    }
}

#[utoipa::path(
    delete,
    path = "/api/v1/projects/{projectId}/shares/{userId}",
    params(
        ("projectId" = String, Path, description = "ID do projeto"),
        ("userId" = String, Path, description = "ID do usuário")
    ),
    responses(
        (status = 204, description = "Compartilhamento removido"),
        (status = 403, description = "Sem permissão para revogar compartilhamento", body = ErrorResponse),
        (status = 404, description = "Projeto ou compartilhamento não encontrado", body = ErrorResponse)
    )
)]
pub async fn delete_project_share(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((project_id, user_id)): Path<(String, String)>,
) -> Response {
    match can_access_project(&state.db, &project_id, &principal, ProjectAccess::Manage).await {
        Ok(true) => {}
        Ok(false) => return forbidden_response("only the stack owner can revoke access"),
        Err(err) => return internal_error_response(format!("failed to authorize project: {err}")),
    }

    match delete_project_share_record(&state.db, &project_id, &user_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => not_found_response("project share not found"),
        Err(err) => internal_error_response(format!("failed to revoke project share: {err}")),
    }
}

#[utoipa::path(
    put,
    path = "/api/v1/projects/{projectId}/visibility",
    params(("projectId" = String, Path, description = "ID do projeto")),
    request_body = ProjectVisibilityUpdateRequest,
    responses(
        (status = 200, description = "Visibilidade atualizada", body = ProjectSharingRecord),
        (status = 403, description = "Sem permissão para alterar visibilidade", body = ErrorResponse),
        (status = 404, description = "Projeto não encontrado", body = ErrorResponse)
    )
)]
pub async fn update_project_visibility(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(project_id): Path<String>,
    Json(payload): Json<ProjectVisibilityUpdateRequest>,
) -> Response {
    match can_access_project(&state.db, &project_id, &principal, ProjectAccess::Manage).await {
        Ok(true) => {}
        Ok(false) => return forbidden_response("only the stack owner can change visibility"),
        Err(err) => return internal_error_response(format!("failed to authorize project: {err}")),
    }

    match update_project_visibility_record(&state.db, &project_id, payload.visibility).await {
        Ok(true) => {}
        Ok(false) => return not_found_response("project not found"),
        Err(err) => return internal_error_response(format!("failed to update visibility: {err}")),
    }

    match load_project_sharing_record(&state.db, &project_id).await {
        Ok(Some(record)) => Json(record).into_response(),
        Ok(None) => not_found_response("project not found"),
        Err(err) => internal_error_response(format!("failed to load project shares: {err}")),
    }
}
