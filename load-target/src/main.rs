use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::{Json, Router, routing::get};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::time::{Duration, sleep};
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

#[derive(Debug, Default)]
struct Counters {
    started_at_ms: u128,
    total_requests: u64,
    total_ok: u64,
    total_errors: u64,
    total_latency_ms: u128,
    per_second: BTreeMap<u64, SecondBucket>,
}

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SecondBucket {
    second: u64,
    requests: u64,
    ok: u64,
    errors: u64,
    avg_latency_ms: f64,
    #[serde(skip)]
    total_latency_ms: u128,
}

#[derive(Debug, Clone)]
struct AppState {
    counters: Arc<Mutex<Counters>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthResponse {
    status: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LoadResponse {
    status: &'static str,
    total_requests: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MetricsResponse {
    started_at_ms: u128,
    elapsed_ms: u128,
    total_requests: u64,
    total_ok: u64,
    total_errors: u64,
    avg_latency_ms: f64,
    current_rps: u64,
    per_second: Vec<SecondBucket>,
}

#[derive(Debug, Deserialize)]
struct SlowQuery {
    ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct FailQuery {
    rate: Option<u64>,
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before epoch")
        .as_millis()
}

fn app_state() -> AppState {
    AppState {
        counters: Arc::new(Mutex::new(Counters {
            started_at_ms: now_ms(),
            ..Counters::default()
        })),
    }
}

fn app() -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/load/ok", get(load_ok))
        .route("/load/slow", get(load_slow))
        .route("/load/fail", get(load_fail))
        .route("/metrics", get(metrics))
        .route("/metrics/reset", get(reset_metrics))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(app_state())
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn load_ok(State(state): State<AppState>) -> Json<LoadResponse> {
    let total_requests = record_request(&state, false, 0);
    Json(LoadResponse {
        status: "ok",
        total_requests,
    })
}

async fn load_slow(
    State(state): State<AppState>,
    Query(query): Query<SlowQuery>,
) -> Json<LoadResponse> {
    let delay_ms = query.ms.unwrap_or(50).min(30_000);
    sleep(Duration::from_millis(delay_ms)).await;
    let total_requests = record_request(&state, false, delay_ms);
    Json(LoadResponse {
        status: "ok",
        total_requests,
    })
}

async fn load_fail(
    State(state): State<AppState>,
    Query(query): Query<FailQuery>,
) -> (StatusCode, Json<LoadResponse>) {
    let rate = query.rate.unwrap_or(100).clamp(1, 100);
    let should_fail = {
        let counters = state.counters.lock().expect("metrics lock");
        ((counters.total_requests + 1) % 100) < rate
    };
    let total_requests = record_request(&state, should_fail, 0);
    let status = if should_fail {
        StatusCode::INTERNAL_SERVER_ERROR
    } else {
        StatusCode::OK
    };
    let body_status = if should_fail { "error" } else { "ok" };
    (
        status,
        Json(LoadResponse {
            status: body_status,
            total_requests,
        }),
    )
}

async fn metrics(State(state): State<AppState>) -> Json<MetricsResponse> {
    Json(snapshot_metrics(&state))
}

async fn reset_metrics(State(state): State<AppState>) -> Json<HealthResponse> {
    let mut counters = state.counters.lock().expect("metrics lock");
    *counters = Counters {
        started_at_ms: now_ms(),
        ..Counters::default()
    };
    Json(HealthResponse { status: "reset" })
}

fn record_request(state: &AppState, failed: bool, latency_ms: u64) -> u64 {
    let now = now_ms();
    let mut counters = state.counters.lock().expect("metrics lock");
    let elapsed_ms = now.saturating_sub(counters.started_at_ms);
    let second = (elapsed_ms / 1000) as u64;

    counters.total_requests += 1;
    counters.total_latency_ms += latency_ms as u128;
    if failed {
        counters.total_errors += 1;
    } else {
        counters.total_ok += 1;
    }

    let total_requests = counters.total_requests;
    let bucket = counters
        .per_second
        .entry(second)
        .or_insert_with(|| SecondBucket {
            second,
            ..SecondBucket::default()
        });
    bucket.requests += 1;
    bucket.total_latency_ms += latency_ms as u128;
    if failed {
        bucket.errors += 1;
    } else {
        bucket.ok += 1;
    }
    bucket.avg_latency_ms = bucket.total_latency_ms as f64 / bucket.requests as f64;

    total_requests
}

fn snapshot_metrics(state: &AppState) -> MetricsResponse {
    let now = now_ms();
    let counters = state.counters.lock().expect("metrics lock");
    let elapsed_ms = now.saturating_sub(counters.started_at_ms);
    let latest_second = counters.per_second.keys().next_back().copied();
    let current_rps = latest_second
        .and_then(|second| {
            counters
                .per_second
                .get(&second)
                .map(|bucket| bucket.requests)
        })
        .unwrap_or(0);

    MetricsResponse {
        started_at_ms: counters.started_at_ms,
        elapsed_ms,
        total_requests: counters.total_requests,
        total_ok: counters.total_ok,
        total_errors: counters.total_errors,
        avg_latency_ms: if counters.total_requests == 0 {
            0.0
        } else {
            counters.total_latency_ms as f64 / counters.total_requests as f64
        },
        current_rps,
        per_second: counters.per_second.values().cloned().collect(),
    }
}

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let address = std::env::var("ADDRESS").unwrap_or_else(|_| "127.0.0.1".to_owned());
    let port = std::env::var("PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(5620);
    let bind_addr = format!("{address}:{port}");

    let listener = TcpListener::bind(&bind_addr)
        .await
        .expect("failed to bind load target listener");
    info!(
        "previa-load-target listening on http://{}",
        listener.local_addr().expect("local addr")
    );

    axum::serve(listener, app())
        .await
        .expect("failed to start load target");
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use serde_json::Value;
    use tower::ServiceExt;

    use super::app;

    async fn json_response(app: Router, uri: &str) -> (StatusCode, Value) {
        let response = app
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let status = response.status();
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let value = serde_json::from_slice::<Value>(&body).expect("json");
        (status, value)
    }

    #[tokio::test]
    async fn ok_endpoint_increments_metrics() {
        let app = app();

        let (status, body) = json_response(app.clone(), "/load/ok").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");

        let (status, metrics) = json_response(app, "/metrics").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(metrics["totalRequests"], 1);
        assert_eq!(metrics["totalOk"], 1);
        assert_eq!(metrics["totalErrors"], 0);
    }

    #[tokio::test]
    async fn reset_endpoint_clears_metrics() {
        let app = app();

        let _ = json_response(app.clone(), "/load/ok").await;
        let (status, reset) = json_response(app.clone(), "/metrics/reset").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(reset["status"], "reset");

        let (_status, metrics) = json_response(app, "/metrics").await;
        assert_eq!(metrics["totalRequests"], 0);
        assert_eq!(
            metrics["perSecond"].as_array().expect("per second").len(),
            0
        );
    }
}
