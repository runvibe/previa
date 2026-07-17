mod server;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::server::build_app;
use crate::server::queue::config::RunnerQueueConfig;
use crate::server::queue::heartbeat::run_heartbeat;
use crate::server::queue::load_executor::QueueJobExecutor;
use crate::server::queue::repository::{RunnerQueueRepository, RunnerRegistration};
use crate::server::queue::worker::RunnerWorker;
use crate::server::state::AppState;

fn should_print_version(args: impl IntoIterator<Item = String>) -> bool {
    args.into_iter()
        .skip(1)
        .any(|arg| arg == "--version" || arg == "-v")
}

fn optional_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[tokio::main]
async fn main() {
    if should_print_version(std::env::args()) {
        println!("previa-runner {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let queue_config = RunnerQueueConfig::from_env().expect("invalid Postgres queue configuration");
    let queue_repository = RunnerQueueRepository::connect(&queue_config.database_url, 8)
        .await
        .expect("failed to connect to Postgres queue");
    let mut registration =
        RunnerRegistration::from_env().expect("invalid runner registration configuration");
    registration.heartbeat_interval = queue_config.heartbeat_interval;
    let identity = queue_repository
        .register(&registration)
        .await
        .expect("failed to register Postgres queue runner");
    let queue_ready = Arc::new(AtomicBool::new(true));
    let worker = RunnerWorker::new(
        Arc::new(queue_repository.clone()),
        Arc::new(QueueJobExecutor::default()),
        identity.clone(),
        queue_config.clone(),
        Duration::from_secs(30),
    )
    .expect("invalid queue lease configuration");
    let queue_cancel = CancellationToken::new();
    let worker_cancel = queue_cancel.clone();
    let worker_ready = queue_ready.clone();
    tokio::spawn(async move {
        if let Err(error) = worker.run(worker_cancel).await {
            tracing::error!("Postgres queue worker stopped: {error}");
            worker_ready.store(false, Ordering::SeqCst);
        }
    });
    tokio::spawn(run_heartbeat(
        queue_repository,
        identity,
        queue_config.heartbeat_interval,
        queue_cancel,
    ));

    let state = AppState {
        runner_auth_key: optional_env("RUNNER_AUTH_KEY"),
        reservation: crate::server::reservation::ReservationState::from_env(),
        queue_ready: Some(queue_ready),
        ..AppState::default()
    };
    let address = std::env::var("ADDRESS").unwrap_or_else(|_| "0.0.0.0".to_owned());
    let port = std::env::var("PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(55880);
    info!("runner startup config: ADDRESS={}, PORT={}", address, port);
    let bind_addr = format!("{}:{}", address, port);

    let app = build_app(state);

    let listener = TcpListener::bind(&bind_addr)
        .await
        .expect("failed to bind listener");
    let local_addr = listener
        .local_addr()
        .expect("failed to read local bind address");

    info!("previa-runner listening on http://{}", local_addr);
    axum::serve(listener, app)
        .await
        .expect("failed to start server");
}

#[cfg(test)]
mod tests {
    use super::should_print_version;

    #[test]
    fn detects_version_flags() {
        assert!(should_print_version(vec![
            "previa-runner".to_owned(),
            "--version".to_owned(),
        ]));
        assert!(should_print_version(vec![
            "previa-runner".to_owned(),
            "-v".to_owned(),
        ]));
        assert!(!should_print_version(vec!["previa-runner".to_owned()]));
    }
}
