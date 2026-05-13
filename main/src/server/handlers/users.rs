use axum::extract::{Extension, Json, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::server::auth::Principal;
use crate::server::auth::passwords::hash_password;
use crate::server::db::{
    UserInsert, UserUpdate, delete_user_record, insert_user_record, list_user_records,
    update_user_record,
};
use crate::server::models::{ErrorResponse, UserCreateRequest, UserUpdateRequest};
use crate::server::state::AppState;
use crate::server::utils::new_uuid_v7;

#[utoipa::path(
    get,
    path = "/api/v1/users",
    responses(
        (status = 200, description = "Managed users", body = Vec<crate::server::models::UserRecord>)
    )
)]
pub async fn list_users(State(state): State<AppState>) -> Response {
    match list_user_records(&state.db).await {
        Ok(users) => Json(users).into_response(),
        Err(err) => internal_error(err.to_string()),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/users",
    request_body = UserCreateRequest,
    responses(
        (status = 201, description = "User created", body = crate::server::models::UserRecord),
        (status = 403, description = "Insufficient permissions", body = ErrorResponse)
    )
)]
pub async fn create_user(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(payload): Json<UserCreateRequest>,
) -> Response {
    if !principal.role.can_create_role(payload.role) {
        return forbidden("cannot create user with requested role");
    }
    let password_hash = match hash_password(&payload.password) {
        Ok(hash) => hash,
        Err(err) => return internal_error(err.to_string()),
    };
    match insert_user_record(
        &state.db,
        UserInsert {
            id: new_uuid_v7(),
            username: payload.username,
            password_hash,
            role: payload.role,
            active: payload.active,
        },
    )
    .await
    {
        Ok(user) => (StatusCode::CREATED, Json(user)).into_response(),
        Err(err) => internal_error(err.to_string()),
    }
}

#[utoipa::path(
    patch,
    path = "/api/v1/users/{userId}",
    request_body = UserUpdateRequest,
    responses(
        (status = 200, description = "User updated", body = crate::server::models::UserRecord),
        (status = 404, description = "User not found", body = ErrorResponse)
    )
)]
pub async fn update_user(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(user_id): Path<String>,
    Json(payload): Json<UserUpdateRequest>,
) -> Response {
    if let Some(role) = payload.role {
        if !principal.role.can_create_role(role) {
            return forbidden("cannot assign requested role");
        }
    }
    let password_hash = match payload.password {
        Some(password) => match hash_password(&password) {
            Ok(hash) => Some(hash),
            Err(err) => return internal_error(err.to_string()),
        },
        None => None,
    };
    match update_user_record(
        &state.db,
        &user_id,
        UserUpdate {
            username: payload.username,
            password_hash,
            role: payload.role,
            active: payload.active,
        },
    )
    .await
    {
        Ok(Some(user)) => Json(user).into_response(),
        Ok(None) => not_found(),
        Err(err) => internal_error(err.to_string()),
    }
}

#[utoipa::path(
    delete,
    path = "/api/v1/users/{userId}",
    responses(
        (status = 204, description = "User deleted"),
        (status = 404, description = "User not found", body = ErrorResponse)
    )
)]
pub async fn delete_user(State(state): State<AppState>, Path(user_id): Path<String>) -> Response {
    match delete_user_record(&state.db, &user_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => not_found(),
        Err(err) => internal_error(err.to_string()),
    }
}

fn forbidden(message: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(ErrorResponse {
            error: "forbidden".to_owned(),
            message: message.to_owned(),
        }),
    )
        .into_response()
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: "not_found".to_owned(),
            message: "user not found".to_owned(),
        }),
    )
        .into_response()
}

fn internal_error(message: String) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: "user_error".to_owned(),
            message,
        }),
    )
        .into_response()
}
