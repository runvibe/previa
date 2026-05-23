use axum::Router;
use axum::http::header;
use axum::middleware::{from_fn, from_fn_with_state};
use axum::routing::{get, post};
use tower_http::cors::{Any, CorsLayer};

use crate::server::handlers::e2e::{rerun_e2e_from_step, run_e2e_test};
use crate::server::handlers::load::{
    ack_load_telemetry, cancel_load_test, get_load_status, get_load_telemetry, run_load_test,
    start_load_test,
};
use crate::server::handlers::system::{
    health, info_runtime, openapi_json, ready, rearm_reservation, release_reservation,
};
use crate::server::middleware::auth::require_runner_authorization;
use crate::server::middleware::http_logging::log_http_io;
use crate::server::middleware::transaction::propagate_transaction_header;
use crate::server::state::AppState;

pub mod docs;
pub mod errors;
pub mod handlers;
pub mod load_dispatch;
pub mod load_execution;
pub mod load_wave;
pub mod metrics;
pub mod middleware;
pub mod models;
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
    let api_v1 = Router::new()
        .route("/tests/e2e", post(run_e2e_test))
        .route("/tests/e2e/rerun-from-step", post(rerun_e2e_from_step))
        .route("/tests/load", post(run_load_test))
        .route("/tests/load/start", post(start_load_test))
        .route(
            "/tests/load/{execution_id}/telemetry",
            get(get_load_telemetry),
        )
        .route(
            "/tests/load/{execution_id}/telemetry/ack",
            post(ack_load_telemetry),
        )
        .route("/tests/load/{execution_id}/status", get(get_load_status))
        .route("/tests/load/{execution_id}/cancel", post(cancel_load_test));
    let protected = Router::new()
        .nest("/api/v1", api_v1)
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/info", get(info_runtime))
        .route("/internal/reservation/rearm", post(rearm_reservation))
        .route("/internal/reservation/release", post(release_reservation))
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
