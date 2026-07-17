use axum::Router;
use axum::http::header;
use axum::middleware::{from_fn, from_fn_with_state};
use axum::routing::get;
use tower_http::cors::{Any, CorsLayer};

use crate::server::handlers::system::{health, info_runtime, openapi_json, ready};
use crate::server::middleware::auth::require_runner_authorization;
use crate::server::middleware::http_logging::log_http_io;
use crate::server::middleware::transaction::propagate_transaction_header;
use crate::server::state::AppState;

pub mod docs;
pub mod errors;
pub mod handlers;
pub mod load_dispatch;
pub mod load_wave;
pub mod metrics;
pub mod middleware;
pub mod models;
pub mod queue;
pub mod reservation;
pub mod runtime;
pub mod sse;
pub mod state;
pub mod utils;
pub mod wave_dispatcher;
pub mod wave_emitter;
pub mod wave_executor;
pub mod wave_metrics_actor;
pub mod wave_scheduler;
pub mod wave_sender;

pub fn build_app(state: AppState) -> Router {
    let protected = Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/info", get(info_runtime))
        .layer(from_fn_with_state(
            state.clone(),
            require_runner_authorization,
        ));

    Router::new()
        .merge(protected)
        .route("/openapi.json", get(openapi_json))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any)
                .expose_headers([header::CONTENT_TYPE]),
        )
        .layer(from_fn(propagate_transaction_header))
        .layer(from_fn(log_http_io))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn execution_http_routes_are_removed_from_router_and_openapi() {
        let app = build_app(AppState::default());
        for path in [
            "/api/v1/tests/e2e",
            "/api/v1/tests/load",
            "/api/v1/tests/load/start",
            "/internal/reservation/rearm",
        ] {
            let response = app
                .clone()
                .oneshot(Request::post(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        }

        let response = app
            .oneshot(Request::get("/openapi.json").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let document = String::from_utf8(body.to_vec()).unwrap();
        assert!(!document.contains("/tests/e2e"));
        assert!(!document.contains("/tests/load"));
        assert!(document.contains("/info"));
    }
}
