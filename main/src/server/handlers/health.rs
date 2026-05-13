use crate::server::docs::build_openapi_document;
use crate::server::models::OrchestratorInfoResponse;
use crate::server::state::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;

#[utoipa::path(
    get,
    path = "/health",
    responses(
        (status = 200, description = "Orchestrator saudável")
    )
)]
pub async fn health() -> StatusCode {
    StatusCode::OK
}

#[utoipa::path(
    get,
    path = "/info",
    responses(
        (status = 200, description = "Runners cadastrados e status de atividade", body = OrchestratorInfoResponse)
    )
)]
pub async fn get_info(State(state): State<AppState>) -> Json<OrchestratorInfoResponse> {
    let runners = crate::server::services::runner_registry::collect_registered_runner_statuses(
        &state.db,
        &state.client,
        state.runner_auth_key.as_deref(),
    )
    .await
    .unwrap_or_default();
    let active_runners = runners.iter().filter(|runner| runner.active).count();

    Json(OrchestratorInfoResponse {
        context: state.context_name.clone(),
        total_runners: runners.len(),
        active_runners,
        runners,
    })
}

pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(build_openapi_document())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use reqwest::Client;
    use tokio::sync::RwLock;

    use crate::server::execution::ExecutionScheduler;
    use crate::server::state::AppState;

    use super::get_info;

    #[tokio::test]
    async fn info_includes_context_name() {
        let db = crate::server::db::DbPool::connect("sqlite::memory:", 1)
            .await
            .expect("sqlite memory db");
        let state = AppState {
            client: Client::new(),
            db,
            context_name: "other".to_owned(),
            runner_auth_key: None,
            auth: crate::server::auth::AuthRuntime::anonymous(),
            rps_per_node: 1000,
            scheduler: ExecutionScheduler::new(Default::default()),
            executions: Arc::new(RwLock::new(HashMap::new())),
            e2e_queues: Arc::new(RwLock::new(HashMap::new())),
            mcp_sessions: Arc::new(RwLock::new(HashMap::new())),
        };

        let response = get_info(axum::extract::State(state)).await;
        let payload = response.0;
        assert_eq!(payload.context, "other");
        assert_eq!(payload.total_runners, 0);
        assert_eq!(payload.active_runners, 0);
        assert!(payload.runners.is_empty());
    }
}
