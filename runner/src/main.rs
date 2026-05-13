mod server;

use tokio::net::TcpListener;
use tracing::info;

use crate::server::build_app;
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

    let state = AppState {
        runner_auth_key: optional_env("RUNNER_AUTH_KEY"),
        reservation: crate::server::reservation::ReservationState::from_env(),
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
