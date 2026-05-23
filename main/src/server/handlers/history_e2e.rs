use axum::extract::{Extension, Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::{Json, http::StatusCode};
use sqlx::QueryBuilder;

use crate::server::auth::Principal;
use crate::server::db::{list_e2e_history_records, load_e2e_history_record_by_id, project_exists};
use crate::server::errors::{forbidden_response, internal_error_response, not_found_response};
use crate::server::models::{E2eHistoryRecord, ErrorResponse, HistoryQuery};
use crate::server::services::pipeline_access::{
    PipelineAccess, can_access_optional_pipeline, is_admin,
};
use crate::server::state::AppState;

#[utoipa::path(
    get,
    path = "/api/v1/projects/{projectId}/tests/e2e",
    params(
        ("projectId" = String, Path, description = "ID do projeto"),
        ("pipelineIndex" = Option<i64>, Query, description = "Filtra por índice da pipeline"),
        ("limit" = Option<u32>, Query, description = "Limite de registros retornados (default 100, max 500)"),
        ("offset" = Option<u32>, Query, description = "Deslocamento da paginação (default 0)"),
        ("order" = Option<crate::server::models::HistoryOrder>, Query, description = "Ordem por finishedAtMs: asc | desc (default desc)")
    ),
    responses(
        (
            status = 200,
            description = "Histórico de execuções de integração",
            body = Vec<E2eHistoryRecord>
        ),
        (
            status = 500,
            description = "Erro ao consultar histórico",
            body = ErrorResponse
        )
    )
)]
pub async fn list_e2e_history(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(project_id): Path<String>,
    Query(query): Query<HistoryQuery>,
) -> Response {
    match list_e2e_history_records(&state.db, &project_id, query).await {
        Ok(records) => {
            let mut visible = Vec::new();
            for record in records {
                match can_access_optional_pipeline(
                    &state.db,
                    &project_id,
                    record.pipeline_id.as_deref(),
                    &principal,
                    PipelineAccess::Read,
                )
                .await
                {
                    Ok(true) => visible.push(record),
                    Ok(false) => {}
                    Err(err) => {
                        return internal_error_response(format!(
                            "failed to authorize history: {err}"
                        ));
                    }
                }
            }
            Json(visible).into_response()
        }
        Err(err) => return internal_error_response(format!("failed to query history: {err}")),
    }
}

#[utoipa::path(
    delete,
    path = "/api/v1/projects/{projectId}/tests/e2e",
    params(
        ("projectId" = String, Path, description = "ID do projeto"),
        ("pipelineIndex" = Option<i64>, Query, description = "Se informado, remove histórico apenas do índice da pipeline")
    ),
    responses(
        (status = 204, description = "Histórico de integração removido"),
        (status = 404, description = "Projeto não encontrado", body = ErrorResponse),
        (status = 500, description = "Erro ao remover histórico", body = ErrorResponse)
    )
)]
pub async fn delete_e2e_history(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(project_id): Path<String>,
    Query(query): Query<HistoryQuery>,
) -> Response {
    match project_exists(&state.db, &project_id).await {
        Ok(false) => return not_found_response("project not found"),
        Ok(true) => {}
        Err(err) => return internal_error_response(format!("failed to load project: {err}")),
    }

    let pipeline_index_filter = query.pipeline_index;
    if pipeline_index_filter.is_none() && !is_admin(&principal) {
        return forbidden_response("pipeline filter is required to delete history");
    }

    let records = match list_e2e_history_records(&state.db, &project_id, query).await {
        Ok(records) => records,
        Err(err) => return internal_error_response(format!("failed to query history: {err}")),
    };
    for record in records {
        match can_access_optional_pipeline(
            &state.db,
            &project_id,
            record.pipeline_id.as_deref(),
            &principal,
            PipelineAccess::Write,
        )
        .await
        {
            Ok(true) => {}
            Ok(false) => return forbidden_response("pipeline history access denied"),
            Err(err) => {
                return internal_error_response(format!("failed to authorize history: {err}"));
            }
        }
    }

    let mut qb =
        QueryBuilder::<sqlx::Any>::new("DELETE FROM integration_history WHERE project_id = ");
    qb.push_bind(&project_id);
    if let Some(pipeline_index) = pipeline_index_filter {
        qb.push(" AND pipeline_index = ").push_bind(pipeline_index);
    }

    match qb.build().execute(&state.db).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => internal_error_response(format!("failed to delete e2e history: {err}")),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{projectId}/tests/e2e/{test_id}",
    params(
        ("projectId" = String, Path, description = "ID do projeto"),
        ("test_id" = String, Path, description = "ID do teste (id do histórico ou execution_id)")
    ),
    responses(
        (
            status = 200,
            description = "Execução individual de integração",
            body = E2eHistoryRecord
        ),
        (
            status = 404,
            description = "Teste não encontrado",
            body = ErrorResponse
        )
    )
)]
pub async fn get_e2e_test_by_id(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((project_id, test_id)): Path<(String, String)>,
) -> Response {
    let record = match load_e2e_history_record_by_id(&state.db, &project_id, &test_id).await {
        Ok(record) => record,
        Err(err) => {
            return internal_error_response(format!("failed to query e2e history: {err}"));
        }
    };

    let Some(record) = record else {
        return not_found_response("e2e test not found");
    };
    match can_access_optional_pipeline(
        &state.db,
        &project_id,
        record.pipeline_id.as_deref(),
        &principal,
        PipelineAccess::Read,
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => return not_found_response("e2e test not found"),
        Err(err) => return internal_error_response(format!("failed to authorize history: {err}")),
    }

    Json(record).into_response()
}

#[utoipa::path(
    delete,
    path = "/api/v1/projects/{projectId}/tests/e2e/{test_id}",
    params(
        ("projectId" = String, Path, description = "ID do projeto"),
        ("test_id" = String, Path, description = "ID do teste (id do histórico ou execution_id)")
    ),
    responses(
        (status = 204, description = "Execução de integração removida"),
        (status = 404, description = "Projeto ou teste não encontrado", body = ErrorResponse),
        (status = 500, description = "Erro ao remover execução", body = ErrorResponse)
    )
)]
pub async fn delete_e2e_test_by_id(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((project_id, test_id)): Path<(String, String)>,
) -> Response {
    match project_exists(&state.db, &project_id).await {
        Ok(false) => return not_found_response("project not found"),
        Ok(true) => {}
        Err(err) => return internal_error_response(format!("failed to load project: {err}")),
    }

    let record = match load_e2e_history_record_by_id(&state.db, &project_id, &test_id).await {
        Ok(record) => record,
        Err(err) => {
            return internal_error_response(format!("failed to query e2e history: {err}"));
        }
    };
    let Some(record) = record else {
        return not_found_response("e2e test not found");
    };
    match can_access_optional_pipeline(
        &state.db,
        &project_id,
        record.pipeline_id.as_deref(),
        &principal,
        PipelineAccess::Write,
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => return forbidden_response("pipeline history access denied"),
        Err(err) => return internal_error_response(format!("failed to authorize history: {err}")),
    }

    match sqlx::query(
        "DELETE FROM integration_history WHERE project_id = ? AND (id = ? OR execution_id = ?)",
    )
    .bind(&project_id)
    .bind(&test_id)
    .bind(&test_id)
    .execute(&state.db)
    .await
    {
        Ok(result) if result.rows_affected() > 0 => StatusCode::NO_CONTENT.into_response(),
        Ok(_) => not_found_response("e2e test not found"),
        Err(err) => internal_error_response(format!("failed to delete e2e history record: {err}")),
    }
}
