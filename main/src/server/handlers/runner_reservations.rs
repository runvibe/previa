use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;

use crate::server::auth::Principal;
use crate::server::db::{
    load_latest_runner_reservation_for_pipeline, load_project_pipeline_record,
};
use crate::server::models::{ErrorResponse, RunnerReservationRecord};
use crate::server::services::pipeline_access::{PipelineAccess, can_access_pipeline};
use crate::server::state::AppState;

#[utoipa::path(
    get,
    path = "/api/v1/projects/{projectId}/pipelines/{pipelineId}/runner-reservation/latest",
    params(
        ("projectId" = String, Path, description = "Project id"),
        ("pipelineId" = String, Path, description = "Pipeline id")
    ),
    responses(
        (status = 200, description = "Latest runner reservation for the pipeline", body = RunnerReservationRecord),
        (status = 404, description = "Pipeline or runner reservation not found", body = ErrorResponse),
        (status = 500, description = "Failed to load runner reservation", body = ErrorResponse)
    )
)]
pub async fn get_latest_runner_reservation_for_pipeline(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((project_id, pipeline_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let pipeline = match load_project_pipeline_record(&state.db, &project_id, &pipeline_id).await {
        Ok(pipeline) => pipeline,
        Err(err) => {
            return internal_error(format!("failed to verify pipeline: {err}")).into_response();
        }
    };

    if pipeline.is_none() {
        return not_found("pipeline not found").into_response();
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
        Ok(false) => return not_found("pipeline not found").into_response(),
        Err(err) => {
            return internal_error(format!("failed to authorize pipeline: {err}")).into_response();
        }
    }

    match load_latest_runner_reservation_for_pipeline(&state.db, &pipeline_id).await {
        Ok(Some(record)) => Json(sanitize_runner_reservation(record)).into_response(),
        Ok(None) => not_found("runner reservation not found").into_response(),
        Err(err) => {
            internal_error(format!("failed to load runner reservation: {err}")).into_response()
        }
    }
}

pub(crate) fn sanitize_runner_reservation(
    mut record: RunnerReservationRecord,
) -> RunnerReservationRecord {
    record.reservation_token = None;
    record
}

fn not_found(message: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: "runner_reservation_not_found".to_owned(),
            message: message.into(),
        }),
    )
}

fn internal_error(message: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: "runner_reservation_error".to_owned(),
            message: message.into(),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::sanitize_runner_reservation;
    use crate::server::models::RunnerReservationRecord;

    #[test]
    fn sanitize_runner_reservation_removes_secret_token() {
        let record = RunnerReservationRecord {
            execution_id: "exec-1".to_owned(),
            pipeline_id: Some("pipe-1".to_owned()),
            capacity_mode: "kubernetes".to_owned(),
            requested_runner_count: 2,
            ready_runner_count: 1,
            target_rps: 1_000,
            node_profile: Some("4gn.nano".to_owned()),
            reservation_id: Some("rr-1".to_owned()),
            reservation_token: Some("secret".to_owned()),
            reservation_expires_at: Some("2026-05-14T10:00:00Z".to_owned()),
            reservation_status: "provisioning".to_owned(),
            runner_endpoints: vec!["http://10.0.0.1:55880".to_owned()],
            created_at: "2026-05-14T09:55:00Z".to_owned(),
            updated_at: "2026-05-14T09:56:00Z".to_owned(),
        };

        let sanitized = sanitize_runner_reservation(record);

        assert!(sanitized.reservation_token.is_none());
        assert_eq!(sanitized.reservation_id.as_deref(), Some("rr-1"));
        assert_eq!(sanitized.ready_runner_count, 1);
    }
}
