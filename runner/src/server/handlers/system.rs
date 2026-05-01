use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use utoipa::OpenApi;

use crate::server::docs::ApiDoc;
use crate::server::models::{ErrorResponse, RunnerInfoResponse};
use crate::server::runtime::snapshot_current_process_runtime;

pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    let mut openapi = ApiDoc::openapi();
    openapi.info.title = env!("CARGO_PKG_NAME").to_owned();
    openapi.info.version = env!("CARGO_PKG_VERSION").to_owned();
    let package_description = env!("CARGO_PKG_DESCRIPTION").trim();
    let package_authors = env!("CARGO_PKG_AUTHORS")
        .split(':')
        .map(str::trim)
        .filter(|author| !author.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    let mut description_parts = Vec::new();
    if !package_description.is_empty() {
        description_parts.push(package_description.to_owned());
    }
    if !package_authors.is_empty() {
        description_parts.push(format!("Authors: {}", package_authors));
    }
    openapi.info.description = if description_parts.is_empty() {
        None
    } else {
        Some(description_parts.join("\n\n"))
    };
    Json(openapi)
}

pub async fn health() -> StatusCode {
    StatusCode::OK
}

#[utoipa::path(
    get,
    path = "/info",
    responses(
        (
            status = 200,
            description = "Uso de recursos do processo do runner",
            body = RunnerInfoResponse
        ),
        (
            status = 503,
            description = "Não foi possível obter métricas do processo",
            body = ErrorResponse
        )
    )
)]
pub async fn info_runtime() -> Response {
    let Some(runtime) = snapshot_current_process_runtime() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "runtime_info_unavailable".to_owned(),
                message: "failed to read process metrics".to_owned(),
            }),
        )
            .into_response();
    };

    Json(runtime).into_response()
}
