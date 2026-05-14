mod models;
mod routes;
mod services;

use std::sync::Arc;

use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::services::config::{CapacityMode, PluginConfig};
use crate::services::kubernetes::KubeRunnerApi;
use crate::services::reconciler::{ReservationReconciler, reconcile_interval_from_env};
use crate::services::reservations::ReservationStore;
use crate::services::runner_health::ReqwestRunnerHealth;

#[tokio::main]
async fn main() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let address = std::env::var("ADDRESS").unwrap_or_else(|_| "0.0.0.0".to_owned());
    let port = std::env::var("PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(55980);
    let bind_addr = format!("{address}:{port}");
    let config = PluginConfig::from_env();
    let kubernetes = if config.capacity_mode == CapacityMode::Kubernetes {
        Some(Arc::new(
            KubeRunnerApi::new(config.clone())
                .await
                .expect("failed to create kubernetes client"),
        )
            as Arc<dyn services::kubernetes::KubernetesRunnerApi>)
    } else {
        None
    };
    let runner_health = Arc::new(ReqwestRunnerHealth::new(reqwest::Client::new()));
    let store = ReservationStore::from_config(config, kubernetes).with_runner_health(runner_health);
    let reconcile_cancel = CancellationToken::new();
    tokio::spawn(
        ReservationReconciler::new(store.clone(), reconcile_interval_from_env())
            .run(reconcile_cancel.clone()),
    );
    let app = routes::build_app(store);

    let listener = TcpListener::bind(&bind_addr)
        .await
        .expect("failed to bind kubernetes plugin");
    info!("previa-kubernetes-plugin listening on {}", bind_addr);
    axum::serve(listener, app)
        .await
        .expect("failed to start kubernetes plugin");
}
