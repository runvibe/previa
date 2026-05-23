use std::time::Duration;

use axum::Router;
use axum::middleware::{from_fn, from_fn_with_state};
use axum::routing::{get, post, put};
use tower_http::cors::{Any, CorsLayer};

use crate::server::handlers::api_tokens::{
    create_api_token, delete_api_token, list_api_tokens, update_api_token,
};
use crate::server::handlers::app::app_fallback;
use crate::server::handlers::auth::{login, me};
use crate::server::handlers::env_groups::{
    create_project_env_group, delete_project_env_group, get_project_env_group,
    list_project_env_groups, upsert_project_env_group,
};
use crate::server::handlers::executions::{
    cancel_execution, stream_execution, stream_execution_events,
};
use crate::server::handlers::health::{get_info, health, openapi_json};
use crate::server::handlers::history_e2e::{
    delete_e2e_history, delete_e2e_test_by_id, get_e2e_test_by_id, list_e2e_history,
};
use crate::server::handlers::history_load::{
    delete_load_history, delete_load_test_by_id, get_load_test_by_id, list_load_history,
};
use crate::server::handlers::pipelines::{
    create_project_pipeline, delete_project_pipeline, get_project_pipeline, list_project_pipelines,
    upsert_project_pipeline,
};
use crate::server::handlers::projects::{
    create_project, delete_project, get_project, list_projects, upsert_project,
};
use crate::server::handlers::proxy::proxy_request;
use crate::server::handlers::runner_reservations::get_latest_runner_reservation_for_pipeline;
use crate::server::handlers::runners::{
    create_runner, delete_runner, get_runner, list_runners, update_runner,
};
use crate::server::handlers::specs::{
    create_project_spec, delete_project_spec, get_project_spec, list_project_specs,
    upsert_project_spec, validate_openapi_spec,
};
use crate::server::handlers::tests_e2e::{
    run_e2e_rerun_from_step_for_project, run_e2e_test_for_project,
};
use crate::server::handlers::tests_e2e_queue::{
    create_e2e_queue_for_project, delete_e2e_queue_for_project, get_current_e2e_queue_for_project,
    get_e2e_queue_for_project,
};
use crate::server::handlers::tests_load::{preview_load_capacity, run_load_test_for_project};
use crate::server::handlers::transfers::{
    export_project, export_projects_sqlite, import_pipelines, import_project,
};
use crate::server::handlers::users::{create_user, delete_user, list_users, update_user};
use crate::server::mcp::handlers::{delete_http_session, get_http, handle_http, preflight};
use crate::server::mcp::models::McpConfig;
use crate::server::middleware::auth::require_client_auth;
use crate::server::middleware::transaction::propagate_transaction_header;
use crate::server::state::AppState;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub enabled: bool,
    pub mcp_path: Option<String>,
}

impl AppConfig {
    #[cfg(test)]
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            mcp_path: None,
        }
    }
}

pub mod auth;
pub mod db;
pub mod docs;
pub mod errors;
pub mod execution;
pub mod handlers;
pub mod mcp;
pub mod middleware;
pub mod models;
pub mod services;
pub mod state;
pub mod utils;
pub mod validation;

#[cfg(test)]
pub fn build_app(state: AppState, mcp_config: &McpConfig) -> Router {
    build_app_with_config(state, mcp_config, AppConfig::disabled())
}

pub fn build_app_with_config(
    state: AppState,
    mcp_config: &McpConfig,
    app_config: AppConfig,
) -> Router {
    let fallback_config = app_config.clone();
    let mut app = Router::new()
        .route("/health", get(health))
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/auth/me", get(me))
        .route(
            "/api/v1/api-tokens",
            get(list_api_tokens).post(create_api_token),
        )
        .route(
            "/api/v1/api-tokens/{tokenId}",
            axum::routing::patch(update_api_token).delete(delete_api_token),
        )
        .route("/api/v1/users", get(list_users).post(create_user))
        .route(
            "/api/v1/users/{userId}",
            axum::routing::patch(update_user).delete(delete_user),
        )
        .route("/info", get(get_info))
        .route("/openapi.json", get(openapi_json))
        .route("/proxy", post(proxy_request).options(preflight))
        .route(
            "/api/v1/executions/{executionId}/cancel",
            post(cancel_execution),
        )
        .route(
            "/api/v1/executions/{executionId}/events",
            get(stream_execution_events),
        )
        .route(
            "/api/v1/projects/{projectId}/executions/{executionId}",
            get(stream_execution),
        )
        .route("/api/v1/projects", get(list_projects))
        .route("/api/v1/projects", post(create_project))
        .route("/api/v1/runners", get(list_runners).post(create_runner))
        .route(
            "/api/v1/runners/{runnerId}",
            get(get_runner).patch(update_runner).delete(delete_runner),
        )
        .route("/api/v1/projects/export", post(export_projects_sqlite))
        .route("/api/v1/projects/import", post(import_project))
        .route("/api/v1/projects/import/pipelines", post(import_pipelines))
        .route("/api/v1/specs/validate", post(validate_openapi_spec))
        .route("/api/v1/projects/{projectId}", get(get_project))
        .route("/api/v1/projects/{projectId}/export", get(export_project))
        .route(
            "/api/v1/projects/{projectId}/specs",
            get(list_project_specs).post(create_project_spec),
        )
        .route(
            "/api/v1/projects/{projectId}/specs/{specId}",
            get(get_project_spec)
                .put(upsert_project_spec)
                .delete(delete_project_spec),
        )
        .route(
            "/api/v1/projects/{projectId}/env-groups",
            get(list_project_env_groups).post(create_project_env_group),
        )
        .route(
            "/api/v1/projects/{projectId}/env-groups/{envGroupId}",
            get(get_project_env_group)
                .put(upsert_project_env_group)
                .delete(delete_project_env_group),
        )
        .route(
            "/api/v1/projects/{projectId}/pipelines",
            get(list_project_pipelines).post(create_project_pipeline),
        )
        .route(
            "/api/v1/projects/{projectId}/pipelines/{pipelineId}",
            get(get_project_pipeline)
                .put(upsert_project_pipeline)
                .delete(delete_project_pipeline),
        )
        .route(
            "/api/v1/projects/{projectId}/pipelines/{pipelineId}/runner-reservation/latest",
            get(get_latest_runner_reservation_for_pipeline),
        )
        .route("/api/v1/projects/{projectId}", put(upsert_project))
        .route(
            "/api/v1/projects/{projectId}",
            axum::routing::delete(delete_project),
        )
        .route(
            "/api/v1/projects/{projectId}/tests/e2e",
            get(list_e2e_history)
                .post(run_e2e_test_for_project)
                .delete(delete_e2e_history),
        )
        .route(
            "/api/v1/projects/{projectId}/tests/e2e/rerun-from-step",
            post(run_e2e_rerun_from_step_for_project),
        )
        .route(
            "/api/v1/projects/{projectId}/tests/e2e/{test_id}",
            get(get_e2e_test_by_id).delete(delete_e2e_test_by_id),
        )
        .route(
            "/api/v1/projects/{projectId}/tests/e2e/queue",
            get(get_current_e2e_queue_for_project).post(create_e2e_queue_for_project),
        )
        .route(
            "/api/v1/projects/{projectId}/tests/e2e/queue/{queueId}",
            get(get_e2e_queue_for_project).delete(delete_e2e_queue_for_project),
        )
        .route(
            "/api/v1/projects/{projectId}/tests/load",
            get(list_load_history)
                .post(run_load_test_for_project)
                .delete(delete_load_history),
        )
        .route(
            "/api/v1/tests/load/capacity-preview",
            post(preview_load_capacity),
        )
        .route(
            "/api/v1/projects/{projectId}/tests/load/{test_id}",
            get(get_load_test_by_id).delete(delete_load_test_by_id),
        );

    if mcp_config.enabled {
        app = app.route(
            &mcp_config.path,
            get(get_http)
                .post(handle_http)
                .delete(delete_http_session)
                .options(preflight),
        );
    }

    app.layer(from_fn_with_state(state.clone(), require_client_auth))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any)
                .allow_private_network(true)
                .expose_headers(Any)
                .max_age(Duration::from_secs(60 * 60)),
        )
        .layer(from_fn(propagate_transaction_header))
        .fallback(move |method, uri| app_fallback(method, uri, fallback_config.clone()))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::body::{Body, to_bytes};
    use axum::http::{HeaderValue, Method, Request, StatusCode};
    use previa_runner::{Pipeline, PipelineStep};
    use reqwest::Client;
    use serde_json::Value;
    use tokio::sync::RwLock;
    use tower::ServiceExt;

    use crate::server::auth::AuthRuntime;
    use crate::server::auth::config::AuthConfig;
    use crate::server::db::{
        insert_project_pipeline, insert_project_spec_record, upsert_project_metadata,
    };
    use crate::server::execution::ExecutionScheduler;
    use crate::server::mcp::models::McpConfig;
    use crate::server::models::{
        ProjectMetadataUpsertRequest, ProjectSpecUpsertRequest, SpecUrlEntry,
    };
    use crate::server::state::AppState;

    use super::build_app;
    use super::{AppConfig, build_app_with_config};

    async fn test_app(mcp_enabled: bool) -> axum::Router {
        let db = crate::server::db::DbPool::connect("sqlite::memory:", 1)
            .await
            .expect("sqlite memory db");
        sqlx::migrate!("./migrations/sqlite")
            .run(db.pool())
            .await
            .expect("migrations");
        let state = AppState {
            client: Client::new(),
            db,
            context_name: "default".to_owned(),
            runner_auth_key: None,
            auth: crate::server::auth::AuthRuntime::anonymous(),
            rps_per_node: 1000,
            scheduler: ExecutionScheduler::new(Default::default()),
            executions: Arc::new(RwLock::new(HashMap::new())),
            e2e_queues: Arc::new(RwLock::new(HashMap::new())),
            mcp_sessions: Arc::new(RwLock::new(HashMap::new())),
        };

        build_app(
            state,
            &McpConfig {
                enabled: mcp_enabled,
                path: "/mcp".to_owned(),
            },
        )
    }

    async fn test_app_with_config(app_config: AppConfig) -> axum::Router {
        let db = crate::server::db::DbPool::connect("sqlite::memory:", 1)
            .await
            .expect("sqlite memory db");
        sqlx::migrate!("./migrations/sqlite")
            .run(db.pool())
            .await
            .expect("migrations");
        let state = AppState {
            client: Client::new(),
            db,
            context_name: "default".to_owned(),
            runner_auth_key: None,
            auth: crate::server::auth::AuthRuntime::anonymous(),
            rps_per_node: 1000,
            scheduler: ExecutionScheduler::new(Default::default()),
            executions: Arc::new(RwLock::new(HashMap::new())),
            e2e_queues: Arc::new(RwLock::new(HashMap::new())),
            mcp_sessions: Arc::new(RwLock::new(HashMap::new())),
        };

        build_app_with_config(
            state,
            &McpConfig {
                enabled: false,
                path: "/mcp".to_owned(),
            },
            app_config,
        )
    }

    async fn test_app_with_auth_and_config(
        auth: AuthRuntime,
        app_config: AppConfig,
    ) -> axum::Router {
        let db = crate::server::db::DbPool::connect("sqlite::memory:", 1)
            .await
            .expect("sqlite memory db");
        sqlx::migrate!("./migrations/sqlite")
            .run(db.pool())
            .await
            .expect("migrations");
        let state = AppState {
            client: Client::new(),
            db,
            context_name: "default".to_owned(),
            runner_auth_key: None,
            auth,
            rps_per_node: 1000,
            scheduler: ExecutionScheduler::new(Default::default()),
            executions: Arc::new(RwLock::new(HashMap::new())),
            e2e_queues: Arc::new(RwLock::new(HashMap::new())),
            mcp_sessions: Arc::new(RwLock::new(HashMap::new())),
        };

        build_app_with_config(
            state,
            &McpConfig {
                enabled: false,
                path: "/mcp".to_owned(),
            },
            app_config,
        )
    }

    async fn test_app_with_auth(auth: AuthRuntime) -> axum::Router {
        let db = crate::server::db::DbPool::connect("sqlite::memory:", 1)
            .await
            .expect("sqlite memory db");
        sqlx::migrate!("./migrations/sqlite")
            .run(db.pool())
            .await
            .expect("migrations");
        let state = AppState {
            client: Client::new(),
            db,
            context_name: "default".to_owned(),
            runner_auth_key: None,
            auth,
            rps_per_node: 1000,
            scheduler: ExecutionScheduler::new(Default::default()),
            executions: Arc::new(RwLock::new(HashMap::new())),
            e2e_queues: Arc::new(RwLock::new(HashMap::new())),
            mcp_sessions: Arc::new(RwLock::new(HashMap::new())),
        };

        build_app(
            state,
            &McpConfig {
                enabled: true,
                path: "/mcp".to_owned(),
            },
        )
    }

    fn protected_auth() -> AuthRuntime {
        let config = AuthConfig::from_env_values(&[
            ("PREVIA_AUTH_ANONYMOUS", "false"),
            ("PREVIA_ROOT_USERNAME", "root"),
            ("PREVIA_ROOT_PASSWORD", "secret"),
            ("PREVIA_JWT_SECRET", "test-jwt-secret"),
        ])
        .expect("protected auth config");
        AuthRuntime::from_config(config).expect("protected auth runtime")
    }

    fn protected_root_jwt(auth: &AuthRuntime) -> String {
        auth.jwt
            .as_ref()
            .expect("jwt")
            .issue(
                "root",
                "root",
                crate::server::auth::permissions::Role::Root,
                "env",
            )
            .expect("issue root jwt")
    }

    #[tokio::test]
    async fn anonymous_login_returns_conflict() {
        let app = test_app_with_auth(AuthRuntime::anonymous()).await;

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"username":"root","password":"secret","clientKind":"app"}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload = serde_json::from_slice::<Value>(&body).expect("json");
        assert_eq!(payload["error"], "auth_disabled");
    }

    #[tokio::test]
    async fn protected_mode_rejects_info_without_bearer() {
        let app = test_app_with_auth(protected_auth()).await;

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/info")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn protected_mode_allows_static_app_shell_without_bearer() {
        let app = test_app_with_auth_and_config(
            protected_auth(),
            AppConfig {
                enabled: true,
                mcp_path: Some("/mcp".to_owned()),
            },
        )
        .await;

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn protected_app_login_returns_jwt() {
        let app = test_app_with_auth(protected_auth()).await;

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"username":"root","password":"secret","clientKind":"app"}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload = serde_json::from_slice::<Value>(&body).expect("json");
        assert_eq!(payload["tokenKind"], "jwt");
        assert!(
            payload["token"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
        assert_eq!(payload["user"]["role"], "root");
    }

    #[tokio::test]
    async fn protected_cli_login_returns_api_token() {
        let app = test_app_with_auth(protected_auth()).await;

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"username":"root","password":"secret","clientKind":"api_token","tokenName":"cli"}"#))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload = serde_json::from_slice::<Value>(&body).expect("json");
        assert_eq!(payload["tokenKind"], "api_token");
        assert!(
            payload["token"]
                .as_str()
                .is_some_and(|value| value.starts_with("pvk_"))
        );
        assert_eq!(payload["record"]["name"], "cli");
    }

    #[tokio::test]
    async fn protected_api_token_can_access_info() {
        let app = test_app_with_auth(protected_auth()).await;
        let login = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"username":"root","password":"secret","clientKind":"api_token","tokenName":"cli"}"#))
                    .expect("request"),
            )
            .await
            .expect("login response");
        assert_eq!(login.status(), StatusCode::OK);
        let body = to_bytes(login.into_body(), usize::MAX).await.expect("body");
        let payload = serde_json::from_slice::<Value>(&body).expect("json");
        let token = payload["token"].as_str().expect("api token");

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/info")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn protected_editor_jwt_cannot_mutate_runners() {
        let auth = protected_auth();
        let token = auth
            .jwt
            .as_ref()
            .expect("jwt")
            .issue(
                "usr_editor",
                "editor",
                crate::server::auth::permissions::Role::Editor,
                "database",
            )
            .expect("issue editor jwt");
        let app = test_app_with_auth(auth).await;

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/runners")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"endpoint":"http://runner.example:55880"}"#))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn protected_root_can_create_user_and_database_user_can_login() {
        let auth = protected_auth();
        let token = protected_root_jwt(&auth);
        let app = test_app_with_auth(auth).await;

        let created = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/users")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"username":"editor","password":"editor-secret","role":"editor","active":true}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(created.status(), StatusCode::CREATED);
        let body = to_bytes(created.into_body(), usize::MAX)
            .await
            .expect("body");
        let user = serde_json::from_slice::<Value>(&body).expect("json");
        assert_eq!(user["username"], "editor");
        assert_eq!(user["role"], "editor");
        assert!(user.get("password").is_none());

        let listed = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/users")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(listed.status(), StatusCode::OK);

        let login = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"username":"editor","password":"editor-secret","clientKind":"app"}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(login.status(), StatusCode::OK);
        let body = to_bytes(login.into_body(), usize::MAX).await.expect("body");
        let payload = serde_json::from_slice::<Value>(&body).expect("json");
        assert_eq!(payload["tokenKind"], "jwt");
        assert_eq!(payload["user"]["role"], "editor");
    }

    #[tokio::test]
    async fn protected_database_user_cannot_change_own_role() {
        let auth = protected_auth();
        let root_token = protected_root_jwt(&auth);
        let app = test_app_with_auth(auth).await;

        let created = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/users")
                    .header("authorization", format!("Bearer {root_token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"username":"admin","password":"admin-secret","role":"admin","active":true}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(created.status(), StatusCode::CREATED);
        let body = to_bytes(created.into_body(), usize::MAX)
            .await
            .expect("body");
        let user = serde_json::from_slice::<Value>(&body).expect("json");
        let user_id = user["id"].as_str().expect("user id");

        let login = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"username":"admin","password":"admin-secret","clientKind":"app"}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("login response");
        assert_eq!(login.status(), StatusCode::OK);
        let body = to_bytes(login.into_body(), usize::MAX).await.expect("body");
        let payload = serde_json::from_slice::<Value>(&body).expect("json");
        let admin_token = payload["token"].as_str().expect("admin jwt");

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::PATCH)
                    .uri(format!("/api/v1/users/{user_id}"))
                    .header("authorization", format!("Bearer {admin_token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"role":"viewer"}"#))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload = serde_json::from_slice::<Value>(&body).expect("json");
        assert_eq!(payload["message"], "cannot change own role");
    }

    #[tokio::test]
    async fn protected_editor_cannot_manage_api_tokens() {
        let auth = protected_auth();
        let token = auth
            .jwt
            .as_ref()
            .expect("jwt")
            .issue(
                "usr_editor",
                "editor",
                crate::server::auth::permissions::Role::Editor,
                "database",
            )
            .expect("issue editor jwt");
        let app = test_app_with_auth(auth).await;

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/api-tokens")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"editor-token","role":"editor"}"#))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn protected_deactivated_api_token_is_rejected() {
        let auth = protected_auth();
        let root_jwt = protected_root_jwt(&auth);
        let app = test_app_with_auth(auth).await;

        let created = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/api-tokens")
                    .header("authorization", format!("Bearer {root_jwt}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"ci","role":"viewer"}"#))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(created.status(), StatusCode::CREATED);
        let body = to_bytes(created.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload = serde_json::from_slice::<Value>(&body).expect("json");
        let token = payload["token"].as_str().expect("api token");
        let token_id = payload["record"]["id"].as_str().expect("token id");

        let disabled = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::PATCH)
                    .uri(format!("/api/v1/api-tokens/{token_id}"))
                    .header("authorization", format!("Bearer {root_jwt}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"active":false}"#))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(disabled.status(), StatusCode::OK);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/info")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn protected_expired_api_token_is_rejected() {
        let auth = protected_auth();
        let root_jwt = protected_root_jwt(&auth);
        let app = test_app_with_auth(auth).await;

        let created = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/api-tokens")
                    .header("authorization", format!("Bearer {root_jwt}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"name":"old-ci","role":"viewer","expiresAt":"2000-01-01T00:00:00Z"}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(created.status(), StatusCode::CREATED);
        let body = to_bytes(created.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload = serde_json::from_slice::<Value>(&body).expect("json");
        let token = payload["token"].as_str().expect("api token");

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/info")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    async fn initialize_mcp_session(app: &axum::Router) -> String {
        let initialize = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"codex-test","version":"1.0"}}}"#,
                    ))
                    .expect("initialize request"),
            )
            .await
            .expect("initialize response");

        assert_eq!(initialize.status(), StatusCode::OK);
        let session_id = initialize
            .headers()
            .get("mcp-session-id")
            .expect("session header")
            .to_str()
            .expect("session header utf8")
            .to_owned();
        let body = to_bytes(initialize.into_body(), usize::MAX)
            .await
            .expect("read initialize body");
        let payload: Value = serde_json::from_slice(&body).expect("parse initialize body");
        assert_eq!(
            payload["result"]["capabilities"]["resources"]["listChanged"],
            Value::Bool(false)
        );

        payload["result"]
            .get("protocolVersion")
            .and_then(Value::as_str)
            .expect("protocol version");

        payload.get("id").expect("response id");

        session_id
    }

    #[tokio::test]
    async fn proxy_preflight_allows_private_network_requests() {
        let app = test_app(false).await;

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::OPTIONS)
                    .uri("/proxy")
                    .header("origin", "https://id-preview.example")
                    .header("access-control-request-method", "POST")
                    .header("access-control-request-headers", "content-type")
                    .header("access-control-request-private-network", "true")
                    .body(Body::empty())
                    .expect("preflight request"),
            )
            .await
            .expect("preflight response");

        assert!(response.status().is_success());
        assert_eq!(
            response
                .headers()
                .get("access-control-allow-private-network"),
            Some(&HeaderValue::from_static("true"))
        );
        assert!(
            response
                .headers()
                .contains_key("access-control-allow-origin")
        );
    }

    #[tokio::test]
    async fn runner_crud_routes_manage_registered_runners() {
        let app = test_app(false).await;

        let created = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/runners")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"endpoint":"runner-api:5590","name":"api-a","enabled":true}"#,
                    ))
                    .expect("runner create request"),
            )
            .await
            .expect("runner create response");

        assert_eq!(created.status(), StatusCode::CREATED);
        let body = to_bytes(created.into_body(), usize::MAX)
            .await
            .expect("read runner create body");
        let runner: Value = serde_json::from_slice(&body).expect("parse runner create body");
        assert_eq!(runner["endpoint"], "http://runner-api:5590");
        assert_eq!(runner["name"], "api-a");
        assert_eq!(runner["enabled"], true);
        let runner_id = runner["id"].as_str().expect("runner id");

        let updated = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::PATCH)
                    .uri(format!("/api/v1/runners/{runner_id}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"enabled":false}"#))
                    .expect("runner update request"),
            )
            .await
            .expect("runner update response");

        assert_eq!(updated.status(), StatusCode::OK);
        let body = to_bytes(updated.into_body(), usize::MAX)
            .await
            .expect("read runner update body");
        let runner: Value = serde_json::from_slice(&body).expect("parse runner update body");
        assert_eq!(runner["enabled"], false);

        let listed = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/runners")
                    .body(Body::empty())
                    .expect("runner list request"),
            )
            .await
            .expect("runner list response");

        assert_eq!(listed.status(), StatusCode::OK);
        let body = to_bytes(listed.into_body(), usize::MAX)
            .await
            .expect("read runner list body");
        let runners: Value = serde_json::from_slice(&body).expect("parse runner list body");
        assert_eq!(runners.as_array().expect("runner list array").len(), 1);

        let deleted = app
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri(format!("/api/v1/runners/{runner_id}"))
                    .body(Body::empty())
                    .expect("runner delete request"),
            )
            .await
            .expect("runner delete response");

        assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn app_routes_do_not_render_when_disabled() {
        let app = test_app_with_config(AppConfig::disabled()).await;

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/")
                    .body(Body::empty())
                    .expect("root request"),
            )
            .await
            .expect("root response");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn app_routes_render_index_and_spa_fallback_when_enabled() {
        let app = test_app_with_config(AppConfig {
            enabled: true,
            mcp_path: Some("/mcp".to_owned()),
        })
        .await;

        for uri in ["/", "/index", "/projects/demo/flows"] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::GET)
                        .uri(uri)
                        .body(Body::empty())
                        .expect("app request"),
                )
                .await
                .expect("app response");

            assert_eq!(response.status(), StatusCode::OK, "{uri}");
            let content_type = response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_owned();
            let body = to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("read app body");
            let body = String::from_utf8(body.to_vec()).expect("app body utf8");
            assert!(content_type.starts_with("text/html"), "{uri}");
            assert!(body.contains("<!doctype html>") || body.contains("<!DOCTYPE html>"));
            assert!(body.contains("id=\"root\""));
        }
    }

    #[tokio::test]
    async fn api_not_found_returns_json_when_app_is_enabled() {
        let app = test_app_with_config(AppConfig {
            enabled: true,
            mcp_path: Some("/mcp".to_owned()),
        })
        .await;

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/not-a-real-route")
                    .body(Body::empty())
                    .expect("api request"),
            )
            .await
            .expect("api response");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read api body");
        let payload: Value = serde_json::from_slice(&body).expect("api error json");

        assert!(content_type.starts_with("application/json"));
        assert_eq!(payload["error"], "not_found");
    }

    #[tokio::test]
    async fn mcp_get_returns_method_not_allowed() {
        let app = test_app(true).await;

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/mcp")
                    .body(Body::empty())
                    .expect("mcp get request"),
            )
            .await
            .expect("mcp get response");

        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn mcp_tools_list_requires_session_header_with_http_400() {
        let app = test_app(true).await;

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#,
                    ))
                    .expect("mcp tools/list request"),
            )
            .await
            .expect("mcp tools/list response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn mcp_initialize_then_tools_list_returns_ok() {
        let app = test_app(true).await;

        let session_id = initialize_mcp_session(&app).await;

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .header("mcp-session-id", &session_id)
                    .header("mcp-protocol-version", "2025-06-18")
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
                    ))
                    .expect("tools/list request"),
            )
            .await
            .expect("tools/list response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read tools/list body");
        let payload: Value = serde_json::from_slice(&body).expect("parse tools/list body");
        assert!(
            payload
                .get("result")
                .and_then(|result| result.get("tools"))
                .and_then(Value::as_array)
                .is_some()
        );
    }

    #[tokio::test]
    async fn mcp_resources_list_and_read_return_project_pipeline_and_spec_json() {
        let db = crate::server::db::DbPool::connect("sqlite::memory:", 1)
            .await
            .expect("sqlite memory db");
        sqlx::migrate!("./migrations/sqlite")
            .run(db.pool())
            .await
            .expect("migrations");

        upsert_project_metadata(
            &db,
            "project-1".to_owned(),
            ProjectMetadataUpsertRequest {
                name: "Users API".to_owned(),
                description: Some("Project resource test".to_owned()),
                tags: Vec::new(),
            },
        )
        .await
        .expect("project upsert");
        insert_project_pipeline(
            &db,
            "project-1",
            Pipeline {
                id: Some("pipe-1".to_owned()),
                name: "Smoke".to_owned(),
                description: Some("Smoke pipeline".to_owned()),
                steps: vec![PipelineStep {
                    id: "step-1".to_owned(),
                    name: "Request".to_owned(),
                    description: None,
                    method: "GET".to_owned(),
                    url: "https://example.com/health".to_owned(),
                    headers: Default::default(),
                    body: None,
                    operation_id: None,
                    delay: None,
                    retry: None,
                    asserts: Vec::new(),
                }],
            },
        )
        .await
        .expect("pipeline insert");
        let spec = insert_project_spec_record(
            &db,
            "project-1",
            ProjectSpecUpsertRequest {
                spec: serde_json::json!({
                    "openapi": "3.1.0",
                    "info": { "title": "Users API", "version": "1.0.0" },
                    "paths": {}
                }),
                url: None,
                slug: Some("users".to_owned()),
                urls: vec![SpecUrlEntry {
                    name: "dev".to_owned(),
                    url: "https://dev.example.com".to_owned(),
                    description: None,
                }],
                servers: Default::default(),
                sync: false,
                live: false,
            },
        )
        .await
        .expect("spec insert");

        let app = build_app(
            AppState {
                client: Client::new(),
                db,
                context_name: "default".to_owned(),
                runner_auth_key: None,
                auth: crate::server::auth::AuthRuntime::anonymous(),
                rps_per_node: 1000,
                scheduler: ExecutionScheduler::new(Default::default()),
                executions: Arc::new(RwLock::new(HashMap::new())),
                e2e_queues: Arc::new(RwLock::new(HashMap::new())),
                mcp_sessions: Arc::new(RwLock::new(HashMap::new())),
            },
            &McpConfig {
                enabled: true,
                path: "/mcp".to_owned(),
            },
        );

        let session_id = initialize_mcp_session(&app).await;

        let list_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .header("mcp-session-id", &session_id)
                    .header("mcp-protocol-version", "2025-06-18")
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","id":2,"method":"resources/list","params":{}}"#,
                    ))
                    .expect("resources/list request"),
            )
            .await
            .expect("resources/list response");

        assert_eq!(list_response.status(), StatusCode::OK);
        let list_body = to_bytes(list_response.into_body(), usize::MAX)
            .await
            .expect("read resources/list body");
        let list_payload: Value = serde_json::from_slice(&list_body).expect("parse resources/list");
        let resources = list_payload["result"]["resources"]
            .as_array()
            .expect("resources array");
        assert!(
            resources
                .iter()
                .any(|resource| { resource["uri"] == "previa://openapi" })
        );
        assert!(
            resources
                .iter()
                .any(|resource| { resource["uri"] == "previa://projects/project-1" })
        );
        assert!(resources.iter().any(|resource| {
            resource["uri"] == "previa://projects/project-1/pipelines/id:pipe-1"
        }));
        assert!(resources.iter().any(|resource| {
            resource["uri"] == format!("previa://projects/project-1/specs/{}", spec.id)
        }));

        let project_read = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .header("mcp-session-id", &session_id)
                    .header("mcp-protocol-version", "2025-06-18")
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","id":3,"method":"resources/read","params":{"uri":"previa://projects/project-1"}}"#,
                    ))
                    .expect("project read request"),
            )
            .await
            .expect("project read response");
        assert_eq!(project_read.status(), StatusCode::OK);
        let project_body = to_bytes(project_read.into_body(), usize::MAX)
            .await
            .expect("read project resource body");
        let project_payload: Value =
            serde_json::from_slice(&project_body).expect("parse project resource");
        let project_text = project_payload["result"]["contents"][0]["text"]
            .as_str()
            .expect("project resource text");
        let project_json: Value =
            serde_json::from_str(project_text).expect("parse embedded project json");
        assert_eq!(project_json["id"], "project-1");
        assert_eq!(project_json["name"], "Users API");

        let pipeline_read = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .header("mcp-session-id", &session_id)
                    .header("mcp-protocol-version", "2025-06-18")
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","id":4,"method":"resources/read","params":{"uri":"previa://projects/project-1/pipelines/id:pipe-1"}}"#,
                    ))
                    .expect("pipeline read request"),
            )
            .await
            .expect("pipeline read response");
        assert_eq!(pipeline_read.status(), StatusCode::OK);
        let pipeline_body = to_bytes(pipeline_read.into_body(), usize::MAX)
            .await
            .expect("read pipeline resource body");
        let pipeline_payload: Value =
            serde_json::from_slice(&pipeline_body).expect("parse pipeline resource");
        let pipeline_text = pipeline_payload["result"]["contents"][0]["text"]
            .as_str()
            .expect("pipeline resource text");
        let pipeline_json: Value =
            serde_json::from_str(pipeline_text).expect("parse embedded pipeline json");
        assert_eq!(pipeline_json["id"], "pipe-1");
        assert_eq!(pipeline_json["name"], "Smoke");

        let spec_uri = format!("previa://projects/project-1/specs/{}", spec.id);
        let spec_read = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .header("mcp-session-id", &session_id)
                    .header("mcp-protocol-version", "2025-06-18")
                    .body(Body::from(format!(
                        r#"{{"jsonrpc":"2.0","id":5,"method":"resources/read","params":{{"uri":"{spec_uri}"}}}}"#
                    )))
                    .expect("spec read request"),
            )
            .await
            .expect("spec read response");
        assert_eq!(spec_read.status(), StatusCode::OK);
        let spec_body = to_bytes(spec_read.into_body(), usize::MAX)
            .await
            .expect("read spec resource body");
        let spec_payload: Value = serde_json::from_slice(&spec_body).expect("parse spec resource");
        let spec_text = spec_payload["result"]["contents"][0]["text"]
            .as_str()
            .expect("spec resource text");
        let spec_json: Value = serde_json::from_str(spec_text).expect("parse embedded spec json");
        assert_eq!(spec_json["id"], spec.id);
        assert_eq!(spec_json["slug"], "users");
    }
}
