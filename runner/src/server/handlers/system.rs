use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::sync::atomic::Ordering;
use utoipa::OpenApi;

use crate::server::docs::ApiDoc;
use crate::server::models::{ErrorResponse, RunnerInfoResponse};
use crate::server::reservation::{ReservationRearmError, ReservationReleaseError};
use crate::server::runtime::snapshot_current_process_runtime;
use crate::server::state::AppState;

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

pub async fn ready(State(state): State<AppState>) -> StatusCode {
    let queue_ready = state
        .queue_ready
        .as_ref()
        .is_none_or(|ready| ready.load(Ordering::SeqCst));
    if state.reservation.is_ready() && queue_ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
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
pub async fn info_runtime(State(state): State<AppState>) -> Response {
    let Some(mut runtime) = snapshot_current_process_runtime() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "runtime_info_unavailable".to_owned(),
                message: "failed to read process metrics".to_owned(),
            }),
        )
            .into_response();
    };
    let reservation = state.reservation.snapshot().await;
    runtime.busy = reservation.busy;
    runtime.started_execution_count = reservation.started_execution_count;
    runtime.last_started_at = reservation.last_started_at;
    runtime.last_finished_at = reservation.last_finished_at;

    Json(runtime).into_response()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReservationRearmRequest {
    pub reservation_id: String,
    pub reservation_token: String,
    pub expires_at: Option<String>,
}

pub async fn rearm_reservation(
    State(state): State<AppState>,
    Json(payload): Json<ReservationRearmRequest>,
) -> Response {
    let expires_at = payload
        .expires_at
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc));

    match state
        .reservation
        .rearm(
            payload.reservation_id,
            payload.reservation_token,
            expires_at,
        )
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(ReservationRearmError::Busy) => (
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: "runner_busy".to_owned(),
                message: "runner is busy and cannot be rearmed".to_owned(),
            }),
        )
            .into_response(),
        Err(ReservationRearmError::ActiveReservation) => (
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: "active_reservation".to_owned(),
                message: "runner already has an unconsumed reservation".to_owned(),
            }),
        )
            .into_response(),
    }
}

pub async fn release_reservation(State(state): State<AppState>) -> Response {
    match state.reservation.release() {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(ReservationReleaseError::Busy) => (
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: "runner_busy".to_owned(),
                message: "runner is busy and cannot release reservation".to_owned(),
            }),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::state::AppState;

    #[tokio::test]
    async fn ready_reports_ok_when_runner_is_idle() {
        let status = ready(State(AppState::default())).await;

        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn ready_reports_unavailable_while_runner_busy() {
        let state = AppState::default();
        state.reservation.mark_execution_started().await;

        let status = ready(State(state)).await;

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn info_includes_busy_and_execution_counters() {
        let state = AppState::default();
        state.reservation.mark_execution_started().await;
        state.reservation.mark_execution_finished().await;

        let response = info_runtime(State(state)).await;
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json payload");

        assert_eq!(payload["startedExecutionCount"], 1);
        assert_eq!(payload["busy"], false);
    }
}
