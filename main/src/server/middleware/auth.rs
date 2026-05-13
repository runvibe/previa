use axum::Json;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Method, Request, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::server::auth::config::AuthMode;
use chrono::{DateTime, Utc};

use crate::server::auth::permissions::Permission;
use crate::server::auth::{Principal, PrincipalSource};
use crate::server::db::{load_api_token_auth_record_by_hash, update_api_token_last_used};
use crate::server::models::ErrorResponse;
use crate::server::state::AppState;

pub async fn require_client_auth(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if state.auth.config.mode() == AuthMode::AnonymousFullAccess
        || is_public(request.method(), request.uri().path())
    {
        return next.run(request).await;
    }

    let bearer = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if let Some(token) = bearer {
        if let Some(principal) = authenticate_bearer(&state, token).await {
            let permission = required_permission(request.method(), request.uri().path());
            if principal.role.allows(permission) {
                let mut request = request;
                request.extensions_mut().insert(principal);
                return next.run(request).await;
            }
            return (
                StatusCode::FORBIDDEN,
                Json(ErrorResponse {
                    error: "forbidden".to_owned(),
                    message: "insufficient permissions".to_owned(),
                }),
            )
                .into_response();
        }
    }

    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorResponse {
            error: "unauthorized".to_owned(),
            message: "missing or invalid bearer token".to_owned(),
        }),
    )
        .into_response()
}

fn is_public(method: &Method, path: &str) -> bool {
    path == "/health" || (*method == Method::POST && path == "/api/v1/auth/login")
}

async fn authenticate_bearer(state: &AppState, token: &str) -> Option<Principal> {
    if let Some(jwt) = state.auth.jwt.as_ref() {
        if let Ok(claims) = jwt.verify(token) {
            return Some(Principal {
                subject: claims.sub,
                username: claims.username,
                role: claims.role,
                source: match claims.source.as_str() {
                    "env" => PrincipalSource::Env,
                    "database" => PrincipalSource::Database,
                    _ => PrincipalSource::Database,
                },
            });
        }
    }

    let Some(api_tokens) = state.auth.api_tokens.as_ref() else {
        return None;
    };
    let Ok(hash) = api_tokens.hash(token) else {
        return None;
    };
    let Ok(Some(record)) = load_api_token_auth_record_by_hash(&state.db, &hash).await else {
        return None;
    };
    if !record.active {
        return None;
    }
    if record
        .expires_at
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .is_some_and(|expires_at| expires_at.with_timezone(&Utc) <= Utc::now())
    {
        return None;
    }
    if !api_tokens.verify(token, &record.token_hash) {
        return None;
    }
    let _ = update_api_token_last_used(&state.db, &record.id).await;
    Some(Principal {
        subject: record.id,
        username: record.name,
        role: record.role,
        source: PrincipalSource::ApiToken,
    })
}

fn required_permission(method: &Method, path: &str) -> Permission {
    if path.starts_with("/api/v1/users") {
        return Permission::ManageUsers;
    }
    if path.starts_with("/api/v1/api-tokens") {
        return Permission::ManageApiTokens;
    }
    if path.starts_with("/api/v1/runners") {
        return match *method {
            Method::GET => Permission::ReadRunners,
            _ => Permission::ManageRunners,
        };
    }
    if path == "/proxy" {
        return Permission::ProxyRequests;
    }
    if path.starts_with("/mcp") {
        return Permission::UseMcp;
    }
    if path.contains("/tests/") || path.contains("/executions/") {
        return match *method {
            Method::GET => Permission::ReadProjects,
            Method::DELETE => Permission::DeleteHistory,
            _ => Permission::RunExecutions,
        };
    }
    match *method {
        Method::GET | Method::HEAD | Method::OPTIONS => Permission::ReadProjects,
        _ => Permission::WriteProjects,
    }
}
