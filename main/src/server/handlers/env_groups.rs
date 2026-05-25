use axum::extract::{Extension, Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Json, http::StatusCode};

use crate::server::auth::Principal;
use crate::server::db::{
    delete_project_env_group_record, insert_project_env_group_record,
    list_project_env_group_records, load_project_env_group_record_by_id, project_exists,
    update_project_env_group_record,
};
use crate::server::errors::{
    bad_request_message_response, forbidden_response, internal_error_response, not_found_response,
};
use crate::server::models::{ErrorResponse, ProjectEnvGroupRecord, ProjectEnvGroupUpsertRequest};
use crate::server::services::project_access::{ProjectAccess, can_access_project};
use crate::server::state::AppState;
use crate::server::validation::env_groups::normalize_env_group_payload;

#[utoipa::path(
    get,
    path = "/api/v1/projects/{projectId}/env-groups",
    params(("projectId" = String, Path, description = "ID do projeto")),
    responses(
        (status = 200, description = "Lista de env groups do projeto", body = Vec<ProjectEnvGroupRecord>),
        (status = 404, description = "Projeto não encontrado", body = ErrorResponse)
    )
)]
pub async fn list_project_env_groups(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(project_id): Path<String>,
) -> Response {
    match project_exists(&state.db, &project_id).await {
        Ok(false) => return not_found_response("project not found"),
        Ok(true) => {}
        Err(err) => return internal_error_response(format!("failed to load project: {err}")),
    }

    match can_access_project(&state.db, &project_id, &principal, ProjectAccess::Read).await {
        Ok(true) => {}
        Ok(false) => return not_found_response("project not found"),
        Err(err) => return internal_error_response(format!("failed to authorize project: {err}")),
    }

    match list_project_env_group_records(&state.db, &project_id).await {
        Ok(groups) => Json(groups).into_response(),
        Err(err) => internal_error_response(format!("failed to list project env groups: {err}")),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{projectId}/env-groups",
    params(("projectId" = String, Path, description = "ID do projeto")),
    request_body = ProjectEnvGroupUpsertRequest,
    responses(
        (status = 201, description = "Env group criado", body = ProjectEnvGroupRecord),
        (status = 400, description = "Payload inválido", body = ErrorResponse),
        (status = 404, description = "Projeto não encontrado", body = ErrorResponse)
    )
)]
pub async fn create_project_env_group(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(project_id): Path<String>,
    Json(payload): Json<ProjectEnvGroupUpsertRequest>,
) -> Response {
    let payload = match normalize_env_group_payload(payload) {
        Ok(payload) => payload,
        Err(message) => return bad_request_message_response(message),
    };

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

    match insert_project_env_group_record(&state.db, &project_id, payload).await {
        Ok(group) => (StatusCode::CREATED, Json(group)).into_response(),
        Err(err) => internal_error_response(format!("failed to create project env group: {err}")),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{projectId}/env-groups/{envGroupId}",
    params(
        ("projectId" = String, Path, description = "ID do projeto"),
        ("envGroupId" = String, Path, description = "ID do env group")
    ),
    responses(
        (status = 200, description = "Env group", body = ProjectEnvGroupRecord),
        (status = 404, description = "Projeto ou env group não encontrado", body = ErrorResponse)
    )
)]
pub async fn get_project_env_group(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((project_id, env_group_id)): Path<(String, String)>,
) -> Response {
    match project_exists(&state.db, &project_id).await {
        Ok(false) => return not_found_response("project not found"),
        Ok(true) => {}
        Err(err) => return internal_error_response(format!("failed to load project: {err}")),
    }

    match can_access_project(&state.db, &project_id, &principal, ProjectAccess::Read).await {
        Ok(true) => {}
        Ok(false) => return not_found_response("project not found"),
        Err(err) => return internal_error_response(format!("failed to authorize project: {err}")),
    }

    match load_project_env_group_record_by_id(&state.db, &project_id, &env_group_id).await {
        Ok(Some(group)) => Json(group).into_response(),
        Ok(None) => not_found_response("project env group not found"),
        Err(err) => internal_error_response(format!("failed to load project env group: {err}")),
    }
}

#[utoipa::path(
    put,
    path = "/api/v1/projects/{projectId}/env-groups/{envGroupId}",
    params(
        ("projectId" = String, Path, description = "ID do projeto"),
        ("envGroupId" = String, Path, description = "ID do env group")
    ),
    request_body = ProjectEnvGroupUpsertRequest,
    responses(
        (status = 200, description = "Env group atualizado", body = ProjectEnvGroupRecord),
        (status = 400, description = "Payload inválido", body = ErrorResponse),
        (status = 404, description = "Projeto ou env group não encontrado", body = ErrorResponse)
    )
)]
pub async fn upsert_project_env_group(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((project_id, env_group_id)): Path<(String, String)>,
    Json(payload): Json<ProjectEnvGroupUpsertRequest>,
) -> Response {
    let payload = match normalize_env_group_payload(payload) {
        Ok(payload) => payload,
        Err(message) => return bad_request_message_response(message),
    };

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

    match update_project_env_group_record(&state.db, &project_id, &env_group_id, payload).await {
        Ok(Some(group)) => Json(group).into_response(),
        Ok(None) => not_found_response("project env group not found"),
        Err(err) => internal_error_response(format!("failed to update project env group: {err}")),
    }
}

#[utoipa::path(
    delete,
    path = "/api/v1/projects/{projectId}/env-groups/{envGroupId}",
    params(
        ("projectId" = String, Path, description = "ID do projeto"),
        ("envGroupId" = String, Path, description = "ID do env group")
    ),
    responses(
        (status = 204, description = "Env group removido"),
        (status = 404, description = "Projeto ou env group não encontrado", body = ErrorResponse)
    )
)]
pub async fn delete_project_env_group(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((project_id, env_group_id)): Path<(String, String)>,
) -> Response {
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

    match delete_project_env_group_record(&state.db, &project_id, &env_group_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => not_found_response("project env group not found"),
        Err(err) => internal_error_response(format!("failed to delete project env group: {err}")),
    }
}
