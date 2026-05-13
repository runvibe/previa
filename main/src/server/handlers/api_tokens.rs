use axum::extract::{Extension, Json, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::server::auth::Principal;
use crate::server::db::{
    ApiTokenInsert, delete_api_token_record, insert_api_token_record, list_api_token_records,
    set_api_token_active,
};
use crate::server::models::{
    ApiTokenCreateRequest, ApiTokenCreateResponse, ApiTokenUpdateRequest, ErrorResponse,
};
use crate::server::state::AppState;
use crate::server::utils::new_uuid_v7;

#[utoipa::path(
    get,
    path = "/api/v1/api-tokens",
    responses(
        (status = 200, description = "API token records", body = Vec<crate::server::models::ApiTokenRecord>)
    )
)]
pub async fn list_api_tokens(State(state): State<AppState>) -> Response {
    match list_api_token_records(&state.db).await {
        Ok(records) => Json(records).into_response(),
        Err(err) => internal_error(err.to_string()),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/api-tokens",
    request_body = ApiTokenCreateRequest,
    responses(
        (status = 201, description = "API token created", body = ApiTokenCreateResponse)
    )
)]
pub async fn create_api_token(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(payload): Json<ApiTokenCreateRequest>,
) -> Response {
    if !principal.role.can_create_role(payload.role) {
        return (
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: "forbidden".to_owned(),
                message: "cannot create token with requested role".to_owned(),
            }),
        )
            .into_response();
    }

    let Some(issuer) = state.auth.api_tokens.as_ref() else {
        return internal_error("api token issuer unavailable".to_owned());
    };
    let issued = match issuer.issue() {
        Ok(issued) => issued,
        Err(err) => return internal_error(err.to_string()),
    };
    let record = match insert_api_token_record(
        &state.db,
        ApiTokenInsert {
            id: new_uuid_v7(),
            name: payload.name,
            token_prefix: issued.prefix,
            token_hash: issued.hash,
            role: payload.role,
            created_by_user_id: Some(principal.subject),
            created_by_username: principal.username,
            expires_at: payload.expires_at,
        },
    )
    .await
    {
        Ok(record) => record,
        Err(err) => return internal_error(err.to_string()),
    };

    (
        StatusCode::CREATED,
        Json(ApiTokenCreateResponse {
            token: issued.raw,
            record,
        }),
    )
        .into_response()
}

#[utoipa::path(
    patch,
    path = "/api/v1/api-tokens/{tokenId}",
    request_body = ApiTokenUpdateRequest,
    responses(
        (status = 200, description = "API token updated", body = crate::server::models::ApiTokenRecord)
    )
)]
pub async fn update_api_token(
    State(state): State<AppState>,
    Path(token_id): Path<String>,
    Json(payload): Json<ApiTokenUpdateRequest>,
) -> Response {
    match set_api_token_active(&state.db, &token_id, payload.active).await {
        Ok(Some(record)) => Json(record).into_response(),
        Ok(None) => not_found(),
        Err(err) => internal_error(err.to_string()),
    }
}

#[utoipa::path(
    delete,
    path = "/api/v1/api-tokens/{tokenId}",
    responses(
        (status = 204, description = "API token deleted")
    )
)]
pub async fn delete_api_token(
    State(state): State<AppState>,
    Path(token_id): Path<String>,
) -> Response {
    match delete_api_token_record(&state.db, &token_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => not_found(),
        Err(err) => internal_error(err.to_string()),
    }
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: "not_found".to_owned(),
            message: "api token not found".to_owned(),
        }),
    )
        .into_response()
}

fn internal_error(message: String) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: "api_token_error".to_owned(),
            message,
        }),
    )
        .into_response()
}
