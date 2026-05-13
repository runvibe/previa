use axum::extract::{Extension, Json, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use subtle::ConstantTimeEq;

use crate::server::auth::config::AuthMode;
use crate::server::auth::passwords::verify_password;
use crate::server::auth::permissions::Role;
use crate::server::auth::{Principal, PrincipalSource};
use crate::server::db::{
    ApiTokenInsert, insert_api_token_record, load_user_auth_record_by_username,
};
use crate::server::models::{
    AuthClientKind, AuthLoginRequest, AuthLoginResponse, AuthPrincipalSource, AuthTokenKind,
    AuthUserResponse, ErrorResponse,
};
use crate::server::state::AppState;
use crate::server::utils::new_uuid_v7;

#[utoipa::path(
    post,
    path = "/api/v1/auth/login",
    request_body = AuthLoginRequest,
    responses(
        (status = 200, description = "Authenticated session or API token", body = AuthLoginResponse),
        (status = 401, description = "Invalid credentials", body = ErrorResponse),
        (status = 409, description = "Auth disabled in anonymous mode", body = ErrorResponse)
    )
)]
pub async fn login(
    State(state): State<AppState>,
    Json(payload): Json<AuthLoginRequest>,
) -> Response {
    if state.auth.config.mode() == AuthMode::AnonymousFullAccess {
        return (
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: "auth_disabled".to_owned(),
                message: "login is not required while anonymous access is enabled".to_owned(),
            }),
        )
            .into_response();
    }

    let Some(authenticated) = authenticate_password(&state, &payload).await else {
        return unauthorized();
    };

    match payload.client_kind {
        AuthClientKind::App => {
            let Some(jwt) = state.auth.jwt.as_ref() else {
                return unauthorized();
            };
            match jwt.issue(
                &authenticated.id,
                &authenticated.username,
                authenticated.role,
                authenticated.jwt_source,
            ) {
                Ok(token) => Json(AuthLoginResponse {
                    token_kind: AuthTokenKind::Jwt,
                    token,
                    expires_at: None,
                    user: Some(authenticated.user_response()),
                    record: None,
                })
                .into_response(),
                Err(err) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "auth_error".to_owned(),
                        message: err.to_string(),
                    }),
                )
                    .into_response(),
            }
        }
        AuthClientKind::ApiToken => {
            let Some(api_tokens) = state.auth.api_tokens.as_ref() else {
                return unauthorized();
            };
            let issued = match api_tokens.issue() {
                Ok(issued) => issued,
                Err(err) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: "auth_error".to_owned(),
                            message: err.to_string(),
                        }),
                    )
                        .into_response();
                }
            };
            let name = payload
                .token_name
                .unwrap_or_else(|| "previa-cli".to_owned());
            let record = match insert_api_token_record(
                &state.db,
                ApiTokenInsert {
                    id: new_uuid_v7(),
                    name,
                    token_prefix: issued.prefix,
                    token_hash: issued.hash,
                    role: authenticated.role,
                    created_by_user_id: Some(authenticated.id),
                    created_by_username: authenticated.username,
                    expires_at: None,
                },
            )
            .await
            {
                Ok(record) => record,
                Err(err) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: "auth_error".to_owned(),
                            message: err.to_string(),
                        }),
                    )
                        .into_response();
                }
            };
            Json(AuthLoginResponse {
                token_kind: AuthTokenKind::ApiToken,
                token: issued.raw,
                expires_at: None,
                user: None,
                record: Some(record),
            })
            .into_response()
        }
    }
}

struct AuthenticatedUser {
    id: String,
    username: String,
    role: Role,
    source: AuthPrincipalSource,
    jwt_source: &'static str,
}

impl AuthenticatedUser {
    fn user_response(&self) -> AuthUserResponse {
        AuthUserResponse {
            id: self.id.clone(),
            username: self.username.clone(),
            role: self.role,
            source: self.source.clone(),
        }
    }
}

async fn authenticate_password(
    state: &AppState,
    payload: &AuthLoginRequest,
) -> Option<AuthenticatedUser> {
    if let (Some(root_username), Some(root_password)) = (
        state.auth.config.root_username.as_deref(),
        state.auth.config.root_password.as_deref(),
    ) {
        if payload.username == root_username && constant_eq(&payload.password, root_password) {
            return Some(AuthenticatedUser {
                id: "root".to_owned(),
                username: root_username.to_owned(),
                role: Role::Root,
                source: AuthPrincipalSource::Env,
                jwt_source: "env",
            });
        }
    }

    let record = load_user_auth_record_by_username(&state.db, &payload.username)
        .await
        .ok()
        .flatten()?;
    if !record.active || !verify_password(&payload.password, &record.password_hash) {
        return None;
    }
    Some(AuthenticatedUser {
        id: record.id,
        username: record.username,
        role: record.role,
        source: AuthPrincipalSource::Database,
        jwt_source: "database",
    })
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/me",
    responses(
        (status = 200, description = "Current authenticated principal", body = AuthUserResponse)
    )
)]
pub async fn me(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
) -> Response {
    if state.auth.config.mode() == AuthMode::AnonymousFullAccess {
        return Json(AuthUserResponse {
            id: "anonymous".to_owned(),
            username: "anonymous".to_owned(),
            role: Role::Anonymous,
            source: AuthPrincipalSource::Anonymous,
        })
        .into_response();
    }

    let Some(Extension(principal)) = principal else {
        return unauthorized();
    };
    Json(AuthUserResponse {
        id: principal.subject,
        username: principal.username,
        role: principal.role,
        source: match principal.source {
            PrincipalSource::Env => AuthPrincipalSource::Env,
            PrincipalSource::Database => AuthPrincipalSource::Database,
            PrincipalSource::ApiToken => AuthPrincipalSource::ApiToken,
        },
    })
    .into_response()
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorResponse {
            error: "unauthorized".to_owned(),
            message: "invalid username or password".to_owned(),
        }),
    )
        .into_response()
}

fn constant_eq(left: &str, right: &str) -> bool {
    left.as_bytes().ct_eq(right.as_bytes()).unwrap_u8() == 1
}
