use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};

use crate::models::{ErrorResponse, ReservationCreateRequest};
use crate::services::reservations::ReservationStore;

pub fn build_app(store: ReservationStore) -> Router {
    Router::new()
        .route("/health", get(|| async { StatusCode::OK }))
        .route("/internal/runner-reservations", post(create_reservation))
        .route(
            "/internal/runner-reservations/{reservationId}",
            get(get_reservation),
        )
        .route(
            "/internal/runner-reservations/{reservationId}/cancel",
            post(cancel_reservation),
        )
        .with_state(store)
}

async fn create_reservation(
    State(store): State<ReservationStore>,
    Json(payload): Json<ReservationCreateRequest>,
) -> Response {
    if payload.count == 0 {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "bad_request".to_owned(),
                message: "count must be greater than zero".to_owned(),
            }),
        )
            .into_response();
    }

    match store.create(payload).await {
        Ok(status) => (StatusCode::ACCEPTED, Json(status)).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "reservation_error".to_owned(),
                message: err.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn get_reservation(
    State(store): State<ReservationStore>,
    Path(reservation_id): Path<String>,
) -> Response {
    match store.get(&reservation_id).await {
        Some(status) => Json(status).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "not_found".to_owned(),
                message: "reservation not found".to_owned(),
            }),
        )
            .into_response(),
    }
}

async fn cancel_reservation(
    State(store): State<ReservationStore>,
    Path(reservation_id): Path<String>,
) -> Response {
    if store.cancel(&reservation_id).await {
        StatusCode::NO_CONTENT.into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "not_found".to_owned(),
                message: "reservation not found".to_owned(),
            }),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use super::build_app;
    use crate::services::reservations::ReservationStore;

    #[tokio::test]
    async fn create_reservation_returns_provisioning_status() {
        let app = build_app(ReservationStore::default());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/internal/runner-reservations")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "executionId": "exec-1",
                            "pipelineId": "pipe-1",
                            "count": 2
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert!(
            payload["reservationId"]
                .as_str()
                .unwrap()
                .starts_with("rr_")
        );
        assert_eq!(payload["status"], "provisioning");
        assert_eq!(payload["requestedRunners"], 2);
        assert_eq!(payload["readyRunners"], 0);
    }
}
