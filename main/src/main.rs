mod server;

use std::collections::HashMap;
use std::sync::Arc;

use reqwest::Client;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::info;

use crate::server::db::{
    DatabaseKind, DbPool, backfill_project_spec_md5_hashes, cancel_stale_e2e_queues,
    seed_env_runner_records,
};
use crate::server::execution::{SchedulerConfig, parse_runner_endpoints};
use crate::server::mcp::models::McpConfig;
use crate::server::state::{AppState, DB_SCHEMA_VERSION};
use crate::server::utils::now_iso;
use crate::server::{AppConfig, build_app_with_config};

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

fn truthy_env(key: &str) -> bool {
    optional_env(key)
        .map(|value| {
            matches!(
                value.as_str(),
                "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON"
            )
        })
        .unwrap_or(false)
}

#[tokio::main]
async fn main() {
    if should_print_version(std::env::args()) {
        println!("previa-main {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let runner_endpoints = parse_runner_endpoints();
    let runner_auth_key = optional_env("RUNNER_AUTH_KEY");
    let mcp_config = McpConfig::from_env();
    let app_config = AppConfig {
        enabled: truthy_env("PREVIA_APP_ENABLED"),
        mcp_path: mcp_config.enabled.then(|| mcp_config.path.clone()),
    };
    let database_url = std::env::var("ORCHESTRATOR_DATABASE_URL")
        .unwrap_or_else(|_| "sqlite://orchestrator.db".to_owned());
    let rps_per_node = std::env::var("RUNNER_RPS_PER_NODE")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(1000);
    let e2e_per_runner_limit = std::env::var("E2E_EXECUTIONS_PER_RUNNER")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(1);
    let load_per_runner_limit = std::env::var("LOAD_EXECUTIONS_PER_RUNNER")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(1);
    let address = std::env::var("ADDRESS").unwrap_or_else(|_| "0.0.0.0".to_owned());
    let port = std::env::var("PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(5588);
    let context_name = std::env::var("PREVIA_CONTEXT").unwrap_or_else(|_| "default".to_owned());
    let bind_addr = format!("{}:{}", address, port);

    let db = DbPool::connect(&database_url, 5)
        .await
        .expect("failed to connect orchestrator database");
    match db.kind() {
        DatabaseKind::Sqlite => {
            sqlx::migrate!("./migrations/sqlite")
                .run(db.pool())
                .await
                .expect("failed to run sqlite orchestrator database migrations");
        }
        DatabaseKind::Postgres => {
            sqlx::migrate!("./migrations/postgres")
                .run(db.pool())
                .await
                .expect("failed to run postgres orchestrator database migrations");
        }
    }
    let backfilled_spec_hashes = backfill_project_spec_md5_hashes(&db)
        .await
        .expect("failed to backfill OpenAPI spec md5 hashes");
    seed_env_runner_records(&db, &runner_endpoints)
        .await
        .expect("failed to seed env runner endpoints");
    let cancelled_stale_queues = cancel_stale_e2e_queues(&db, &now_iso())
        .await
        .expect("failed to cancel stale e2e queues");

    let state = AppState {
        client: Client::new(),
        db,
        context_name: context_name.clone(),
        runner_auth_key,
        auth: crate::server::auth::AuthRuntime::from_config(
            crate::server::auth::config::AuthConfig::from_env()
                .expect("invalid auth configuration"),
        )
        .expect("invalid auth runtime"),
        rps_per_node,
        scheduler: crate::server::execution::ExecutionScheduler::new(SchedulerConfig {
            e2e_per_runner_limit,
            load_per_runner_limit,
        }),
        executions: Arc::new(RwLock::new(HashMap::new())),
        e2e_queues: Arc::new(RwLock::new(HashMap::new())),
        mcp_sessions: Arc::new(RwLock::new(HashMap::new())),
    };

    let app = build_app_with_config(state, &mcp_config, app_config);

    let listener = TcpListener::bind(&bind_addr)
        .await
        .expect("failed to bind orchestrator listener");
    let local_addr = listener
        .local_addr()
        .expect("failed to read local bind address");

    info!(
        "previa-main listening on http://{} (context: {}, database: {}, schema_version: {})",
        local_addr, context_name, database_url, DB_SCHEMA_VERSION
    );
    if mcp_config.enabled {
        info!(
            "mcp server enabled at http://{}{}",
            local_addr, mcp_config.path
        );
    }
    if backfilled_spec_hashes > 0 {
        info!(
            "backfilled {} OpenAPI specs without md5 hash",
            backfilled_spec_hashes
        );
    }
    if cancelled_stale_queues > 0 {
        info!(
            "cancelled {} stale e2e queues from previous startup",
            cancelled_stale_queues
        );
    }
    info!(
        "execution scheduler configured (e2e_per_runner_limit: {}, load_per_runner_limit: {})",
        e2e_per_runner_limit, load_per_runner_limit
    );

    axum::serve(listener, app)
        .await
        .expect("failed to start orchestrator");
}

#[cfg(test)]
mod tests {
    use super::should_print_version;

    #[test]
    fn detects_version_flags() {
        assert!(should_print_version(vec![
            "previa-main".to_owned(),
            "--version".to_owned(),
        ]));
        assert!(should_print_version(vec![
            "previa-main".to_owned(),
            "-v".to_owned(),
        ]));
        assert!(!should_print_version(vec!["previa-main".to_owned()]));
    }
}
