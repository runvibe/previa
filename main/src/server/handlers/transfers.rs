use std::path::PathBuf;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State, rejection::JsonRejection};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::server::db::{
    import_project_bundle, load_e2e_history_for_export, load_load_history_for_export,
    load_project_export, project_exists,
};
use crate::server::errors::{
    bad_request_message_response, bad_request_response, conflict_response, internal_error_response,
    not_found_response,
};
use crate::server::models::{
    ErrorResponse, PipelineImportRequest, PipelineImportResponse, ProjectExportEnvelope,
    ProjectImportResponse, ProjectSqliteExportRequest, ProjectTransferQuery,
};
use crate::server::services::pipeline_import::{PipelineImportError, import_pipelines_as_project};
use crate::server::services::sqlite_transfer::{
    export_projects_to_sqlite, import_projects_from_sqlite,
};
use crate::server::state::AppState;
use crate::server::utils::{new_uuid_v7, now_iso};

const PROJECT_EXPORT_FORMAT: &str = "previa.project.export.v1";

#[utoipa::path(
    get,
    path = "/api/v1/projects/{projectId}/export",
    params(
        ("projectId" = String, Path, description = "Project ID"),
        ("includeHistory" = Option<bool>, Query, description = "Include e2e/load history in export. Default true.")
    ),
    responses(
        (
            status = 200,
            description = "Project export bundle",
            body = ProjectExportEnvelope
        ),
        (
            status = 404,
            description = "Project not found",
            body = ErrorResponse
        ),
        (
            status = 500,
            description = "Failed to export project",
            body = ErrorResponse
        )
    )
)]
pub async fn export_project(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    Query(query): Query<ProjectTransferQuery>,
) -> Response {
    let project_id = project_id.trim().to_owned();
    if project_id.is_empty() {
        return bad_request_message_response("projectId cannot be empty");
    }

    let include_history = query.include_history.unwrap_or(true);
    let mut project = match load_project_export(&state.db, &project_id).await {
        Ok(Some(project)) => project,
        Ok(None) => return not_found_response("project not found"),
        Err(err) => {
            return internal_error_response(format!("failed to load project export: {err}"));
        }
    };

    if include_history {
        let e2e = match load_e2e_history_for_export(&state.db, &project_id).await {
            Ok(items) => items,
            Err(err) => {
                return internal_error_response(format!(
                    "failed to load e2e history export: {err}"
                ));
            }
        };
        let load = match load_load_history_for_export(&state.db, &project_id).await {
            Ok(items) => items,
            Err(err) => {
                return internal_error_response(format!(
                    "failed to load load history export: {err}"
                ));
            }
        };
        project.history.e2e = e2e;
        project.history.load = load;
    }

    Json(ProjectExportEnvelope {
        format: PROJECT_EXPORT_FORMAT.to_owned(),
        exported_at: now_iso(),
        history_included: include_history,
        project,
    })
    .into_response()
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/import",
    params(
        ("includeHistory" = Option<bool>, Query, description = "Persist e2e/load history from payload. Default true.")
    ),
    request_body = ProjectExportEnvelope,
    responses(
        (
            status = 201,
            description = "Project imported",
            body = ProjectImportResponse
        ),
        (
            status = 400,
            description = "Invalid payload or format",
            body = ErrorResponse
        ),
        (
            status = 409,
            description = "Import conflict",
            body = ErrorResponse
        ),
        (
            status = 500,
            description = "Failed to import project",
            body = ErrorResponse
        )
    )
)]
pub async fn import_project(
    State(state): State<AppState>,
    Query(query): Query<ProjectTransferQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let include_history = query.include_history.unwrap_or(true);
    if is_sqlite_content(&headers) {
        return import_sqlite_projects(state, include_history, body).await;
    }

    let mut payload = match serde_json::from_slice::<ProjectExportEnvelope>(&body) {
        Ok(payload) => payload,
        Err(err) => {
            let message = format!("invalid JSON payload: {err}");
            return bad_request_message_response(&message);
        }
    };

    if payload.format != PROJECT_EXPORT_FORMAT {
        return bad_request_message_response("invalid import format");
    }

    payload.project.id = payload.project.id.trim().to_owned();
    payload.project.name = payload.project.name.trim().to_owned();
    if payload.project.id.is_empty() {
        return bad_request_message_response("project.id is required");
    }
    if payload.project.name.is_empty() {
        return bad_request_message_response("project.name is required");
    }

    match project_exists(&state.db, &payload.project.id).await {
        Ok(true) => return conflict_response("project already exists"),
        Ok(false) => {}
        Err(err) => return internal_error_response(format!("failed to load project: {err}")),
    }

    match import_project_bundle(&state.db, &payload.project, include_history).await {
        Ok(response) => (StatusCode::CREATED, Json(response)).into_response(),
        Err(sqlx::Error::Database(db_err)) if db_err.is_unique_violation() => {
            conflict_response("import data conflicts with existing records")
        }
        Err(err) => internal_error_response(format!("failed to import project: {err}")),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/export",
    request_body = ProjectSqliteExportRequest,
    responses(
        (
            status = 200,
            description = "SQLite database containing selected projects"
        ),
        (
            status = 400,
            description = "Invalid export selection",
            body = ErrorResponse
        ),
        (
            status = 500,
            description = "Failed to export projects",
            body = ErrorResponse
        )
    )
)]
pub async fn export_projects_sqlite(
    State(state): State<AppState>,
    payload: Result<Json<ProjectSqliteExportRequest>, JsonRejection>,
) -> Response {
    let Json(payload) = match payload {
        Ok(payload) => payload,
        Err(rejection) => return bad_request_response(rejection),
    };

    if payload.all && !payload.project_ids.is_empty() {
        return bad_request_message_response("all=true cannot be combined with projectIds");
    }
    if !payload.all && payload.project_ids.is_empty() {
        return bad_request_message_response("set all=true or provide projectIds");
    }

    let project_ids = payload
        .project_ids
        .iter()
        .map(|project_id| project_id.trim().to_owned())
        .filter(|project_id| !project_id.is_empty())
        .collect::<Vec<_>>();
    if !payload.all && project_ids.is_empty() {
        return bad_request_message_response("projectIds cannot be empty");
    }

    let include_history = payload.include_history.unwrap_or(true);
    let path = temp_sqlite_path("previa-projects-export");
    if let Err(err) =
        export_projects_to_sqlite(&state.db, &path, &project_ids, include_history).await
    {
        let _ = std::fs::remove_file(&path);
        return internal_error_response(format!("failed to export projects to sqlite: {err}"));
    }

    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) => {
            let _ = std::fs::remove_file(&path);
            return internal_error_response(format!("failed to read sqlite export: {err}"));
        }
    };
    let _ = std::fs::remove_file(&path);

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/vnd.sqlite3"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"previa-projects.sqlite3\"",
            ),
        ],
        bytes,
    )
        .into_response()
}

async fn import_sqlite_projects(state: AppState, include_history: bool, body: Bytes) -> Response {
    if body.is_empty() {
        return bad_request_message_response("sqlite import body cannot be empty");
    }

    let path = temp_sqlite_path("previa-projects-import");
    if let Err(err) = std::fs::write(&path, &body) {
        return internal_error_response(format!("failed to write sqlite import: {err}"));
    }

    let response = import_projects_from_sqlite(&state.db, &path, include_history).await;
    let _ = std::fs::remove_file(&path);
    match response {
        Ok(response) => (StatusCode::CREATED, Json(response)).into_response(),
        Err(err) => internal_error_response(format!("failed to import sqlite projects: {err}")),
    }
}

fn is_sqlite_content(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|content_type| content_type.split(';').next().unwrap_or("").trim())
        .is_some_and(|content_type| {
            matches!(
                content_type,
                "application/vnd.sqlite3" | "application/x-sqlite3" | "application/octet-stream"
            )
        })
}

fn temp_sqlite_path(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{prefix}-{}.sqlite3", new_uuid_v7()))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/import/pipelines",
    request_body = PipelineImportRequest,
    responses(
        (
            status = 201,
            description = "Pipelines imported into a newly created project",
            body = PipelineImportResponse
        ),
        (
            status = 400,
            description = "Invalid pipeline payload",
            body = ErrorResponse
        ),
        (
            status = 409,
            description = "Import conflict",
            body = ErrorResponse
        ),
        (
            status = 500,
            description = "Failed to import pipelines",
            body = ErrorResponse
        )
    )
)]
pub async fn import_pipelines(
    State(state): State<AppState>,
    payload: Result<Json<PipelineImportRequest>, JsonRejection>,
) -> Response {
    let Json(payload) = match payload {
        Ok(payload) => payload,
        Err(rejection) => return bad_request_response(rejection),
    };

    match import_pipelines_as_project(&state.db, payload.stack_name, payload.pipelines).await {
        Ok(response) => (StatusCode::CREATED, Json(response)).into_response(),
        Err(PipelineImportError::EmptyStackName) => {
            bad_request_message_response("stackName is required")
        }
        Err(PipelineImportError::EmptyPipelines) => {
            bad_request_message_response("at least one pipeline is required")
        }
        Err(PipelineImportError::EmptyPipelineName(index)) => {
            bad_request_message_response(&format!("pipeline #{index} name is required"))
        }
        Err(PipelineImportError::DuplicatePipelineId(pipeline_id)) => conflict_response(&format!(
            "duplicate pipeline id '{pipeline_id}' in import payload"
        )),
        Err(PipelineImportError::ExistingPipelineId(pipeline_id)) => {
            conflict_response(&format!("pipeline id '{pipeline_id}' already exists"))
        }
        Err(PipelineImportError::ProjectExists(stack_name)) => {
            conflict_response(&format!("project '{stack_name}' already exists"))
        }
        Err(PipelineImportError::Validation(message)) => bad_request_message_response(&message),
        Err(PipelineImportError::Database(err)) => {
            internal_error_response(format!("failed to import pipelines: {err}"))
        }
    }
}
