use axum::{Json, Router, routing::get};
use serde::Serialize;
use tokio::net::TcpListener;
use tracing::info;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthResponse {
    status: &'static str,
}

fn app() -> Router {
    Router::new().route("/health", get(|| async { Json(HealthResponse { status: "ok" }) }))
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
