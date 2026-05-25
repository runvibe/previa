use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Json, http::StatusCode};

use crate::server::auth::Principal;
use crate::server::db::{
    delete_project_spec_record, insert_project_spec_record, list_project_spec_records,
    load_project_spec_record_by_id, project_exists, update_project_spec_record,
};
use crate::server::errors::{
    bad_request_message_response, bad_request_response, forbidden_response,
    internal_error_response, not_found_response,
};
use crate::server::models::{
    ErrorResponse, OpenApiValidationRequest, OpenApiValidationResponse, ProjectSpecRecord,
    ProjectSpecUpsertRequest,
};
use crate::server::services::project_access::{ProjectAccess, can_access_project};
use crate::server::state::AppState;
use crate::server::validation::openapi::validate_openapi_source;
use crate::server::validation::specs::{normalize_spec_slug, normalize_spec_urls_with_legacy};

#[utoipa::path(
    post,
    path = "/api/v1/specs/validate",
    request_body = OpenApiValidationRequest,
    responses(
        (
            status = 200,
            description = "Resultado da validação de spec OpenAPI",
            body = OpenApiValidationResponse
        ),
        (
            status = 400,
            description = "Payload inválido",
            body = ErrorResponse
        )
    )
)]
pub async fn validate_openapi_spec(
    payload: Result<Json<OpenApiValidationRequest>, JsonRejection>,
) -> Response {
    let Json(payload) = match payload {
        Ok(payload) => payload,
        Err(rejection) => return bad_request_response(rejection),
    };

    let result = validate_openapi_source(&payload.source);
    Json(result).into_response()
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{projectId}/specs",
    params(
        ("projectId" = String, Path, description = "ID do projeto")
    ),
    responses(
        (
            status = 200,
            description = "Lista de specs OpenAPI do projeto",
            body = Vec<ProjectSpecRecord>
        ),
        (
            status = 404,
            description = "Projeto não encontrado",
            body = ErrorResponse
        )
    )
)]
pub async fn list_project_specs(
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

    match list_project_spec_records(&state.db, &project_id).await {
        Ok(specs) => Json(specs).into_response(),
        Err(err) => internal_error_response(format!("failed to list project specs: {err}")),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{projectId}/specs",
    params(
        ("projectId" = String, Path, description = "ID do projeto")
    ),
    request_body = ProjectSpecUpsertRequest,
    responses(
        (
            status = 201,
            description = "Spec OpenAPI criada",
            body = ProjectSpecRecord
        ),
        (
            status = 404,
            description = "Projeto não encontrado",
            body = ErrorResponse
        )
    )
)]
pub async fn create_project_spec(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(project_id): Path<String>,
    Json(mut payload): Json<ProjectSpecUpsertRequest>,
) -> Response {
    let normalized_slug = match normalize_spec_slug(payload.slug.as_deref()) {
        Ok(value) => value,
        Err(message) => return bad_request_message_response(message),
    };
    payload.slug = normalized_slug;
    payload.urls =
        match normalize_spec_urls_with_legacy(payload.urls, std::mem::take(&mut payload.servers)) {
            Ok(urls) => urls,
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

    match insert_project_spec_record(&state.db, &project_id, payload).await {
        Ok(spec) => (StatusCode::CREATED, Json(spec)).into_response(),
        Err(err) => internal_error_response(format!("failed to create project spec: {err}")),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{projectId}/specs/{specId}",
    params(
        ("projectId" = String, Path, description = "ID do projeto"),
        ("specId" = String, Path, description = "ID do spec")
    ),
    responses(
        (status = 200, description = "Spec OpenAPI", body = ProjectSpecRecord),
        (status = 404, description = "Projeto ou spec não encontrado", body = ErrorResponse)
    )
)]
pub async fn get_project_spec(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((project_id, spec_id)): Path<(String, String)>,
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

    match load_project_spec_record_by_id(&state.db, &project_id, &spec_id).await {
        Ok(Some(spec)) => Json(spec).into_response(),
        Ok(None) => not_found_response("project spec not found"),
        Err(err) => internal_error_response(format!("failed to load project spec: {err}")),
    }
}

#[utoipa::path(
    put,
    path = "/api/v1/projects/{projectId}/specs/{specId}",
    params(
        ("projectId" = String, Path, description = "ID do projeto"),
        ("specId" = String, Path, description = "ID do spec")
    ),
    request_body = ProjectSpecUpsertRequest,
    responses(
        (status = 200, description = "Spec OpenAPI atualizada", body = ProjectSpecRecord),
        (status = 404, description = "Projeto ou spec não encontrado", body = ErrorResponse)
    )
)]
pub async fn upsert_project_spec(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((project_id, spec_id)): Path<(String, String)>,
    Json(mut payload): Json<ProjectSpecUpsertRequest>,
) -> Response {
    let normalized_slug = match normalize_spec_slug(payload.slug.as_deref()) {
        Ok(value) => value,
        Err(message) => return bad_request_message_response(message),
    };
    payload.slug = normalized_slug;
    payload.urls =
        match normalize_spec_urls_with_legacy(payload.urls, std::mem::take(&mut payload.servers)) {
            Ok(urls) => urls,
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

    match update_project_spec_record(&state.db, &project_id, &spec_id, payload).await {
        Ok(Some(spec)) => Json(spec).into_response(),
        Ok(None) => not_found_response("project spec not found"),
        Err(err) => internal_error_response(format!("failed to update project spec: {err}")),
    }
}

#[utoipa::path(
    delete,
    path = "/api/v1/projects/{projectId}/specs/{specId}",
    params(
        ("projectId" = String, Path, description = "ID do projeto"),
        ("specId" = String, Path, description = "ID do spec")
    ),
    responses(
        (status = 204, description = "Spec OpenAPI removida"),
        (status = 404, description = "Projeto ou spec não encontrado", body = ErrorResponse)
    )
)]
pub async fn delete_project_spec(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((project_id, spec_id)): Path<(String, String)>,
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

    match delete_project_spec_record(&state.db, &project_id, &spec_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => not_found_response("project spec not found"),
        Err(err) => internal_error_response(format!("failed to delete project spec: {err}")),
    }
}
