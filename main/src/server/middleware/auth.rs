use axum::Json;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Method, Request, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::server::auth::config::AuthMode;
use chrono::{DateTime, Utc};

use crate::server::auth::permissions::Permission;
use crate::server::auth::{
    Principal, PrincipalSource, anonymous_full_access_principal, anonymous_principal,
};
use crate::server::db::{load_api_token_auth_record_by_hash, update_api_token_last_used};
use crate::server::models::ErrorResponse;
use crate::server::state::AppState;

pub async fn require_client_auth(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if state.auth.config.mode() == AuthMode::AnonymousFullAccess {
        let mut request = request;
        request
            .extensions_mut()
            .insert(anonymous_full_access_principal());
        return next.run(request).await;
    }

    if is_public(request.method(), request.uri().path()) {
        let mut request = request;
        request.extensions_mut().insert(anonymous_principal());
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

    if allows_anonymous_pipeline_candidate(request.method(), request.uri().path()) {
        let mut request = request;
        request.extensions_mut().insert(anonymous_principal());
        return next.run(request).await;
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

fn allows_anonymous_pipeline_candidate(method: &Method, path: &str) -> bool {
    if let Some(rest) = path.strip_prefix("/api/v1/executions/") {
        return (*method == Method::GET && rest.ends_with("/events"))
            || (*method == Method::POST && rest.ends_with("/cancel"));
    }

    let parts = path
        .trim_matches('/')
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() < 4 || parts[0] != "api" || parts[1] != "v1" || parts[2] != "projects" {
        return false;
    }
    if parts.len() == 4 {
        return *method == Method::GET;
    }
    match parts[4] {
        "pipelines" => {
            (*method == Method::GET && parts.len() == 5)
                || (parts.len() >= 6
                    && matches!(*method, Method::GET | Method::PUT | Method::DELETE))
        }
        "tests" => matches!(*method, Method::GET | Method::POST | Method::DELETE),
        "executions" => *method == Method::GET,
        _ => false,
    }
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
