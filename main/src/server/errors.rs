use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::server::models::ErrorResponse;

pub fn bad_request_response(rejection: JsonRejection) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            error: "bad_request".to_owned(),
            message: rejection.to_string(),
        }),
    )
        .into_response()
}

pub fn bad_request_message_response(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            error: "bad_request".to_owned(),
            message: message.to_owned(),
        }),
    )
        .into_response()
}

pub fn service_unavailable_response(message: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(ErrorResponse {
            error: "service_unavailable".to_owned(),
            message: message.to_owned(),
        }),
    )
        .into_response()
}

pub fn not_found_response(message: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: "not_found".to_owned(),
            message: message.to_owned(),
        }),
    )
        .into_response()
}

pub fn conflict_response(message: &str) -> Response {
    (
        StatusCode::CONFLICT,
        Json(ErrorResponse {
            error: "conflict".to_owned(),
            message: message.to_owned(),
        }),
    )
        .into_response()
}

pub fn forbidden_response(message: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(ErrorResponse {
            error: "forbidden".to_owned(),
            message: message.to_owned(),
        }),
    )
        .into_response()
}

pub fn internal_error_response(message: String) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: "internal_server_error".to_owned(),
            message,
        }),
    )
        .into_response()
}
