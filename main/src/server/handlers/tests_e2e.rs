use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::Response;

use crate::server::db::load_project_pipeline_for_execution;
use crate::server::errors::{
    bad_request_message_response, bad_request_response, internal_error_response,
};
use crate::server::execution::{
    StartE2eExecutionError, sse_response_for_started_execution, start_e2e_execution,
};
use crate::server::middleware::transaction::extract_transaction_id;
use crate::server::models::{
    E2eTestRequest, ErrorResponse, OrchestratorSseEventData, ProjectE2eRerunFromStepRequest,
    ProjectE2eTestRequest,
};
use crate::server::state::AppState;

pub async fn run_e2e_test_internal(
    State(state): State<AppState>,
    project_id: String,
    headers: HeaderMap,
    payload: Result<Json<E2eTestRequest>, JsonRejection>,
) -> Response {
    let Json(payload) = match payload {
        Ok(payload) => payload,
        Err(rejection) => return bad_request_response(rejection),
    };
    let transaction_id = extract_transaction_id(&headers);
    match start_e2e_execution(state, payload, transaction_id).await {
        Ok(started) => {
            let execution_id = started.execution_id.clone();
            response_with_execution_headers(
                project_id,
                &execution_id,
                sse_response_for_started_execution(started),
            )
        }
        Err(StartE2eExecutionError::BadRequest(message)) => bad_request_message_response(&message),
        Err(StartE2eExecutionError::ServiceUnavailable(message)) => {
            crate::server::errors::service_unavailable_response(&message)
        }
        Err(StartE2eExecutionError::Internal(message)) => internal_error_response(message),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{projectId}/tests/e2e",
    params(
        ("projectId" = String, Path, description = "ID do projeto"),
        ("x-transaction-id" = Option<String>, Header, description = "ID de transação para rastreamento; será propagado para os runners e ecoado no response")
    ),
    request_body = ProjectE2eTestRequest,
    responses(
        (
            status = 200,
            description = "Stream SSE unificado para teste de integração (project-scoped).",
            content_type = "text/event-stream",
            body = OrchestratorSseEventData,
            headers(
                ("x-execution-id" = String, description = "ID da execução iniciada para reconexão via GET /executions/{executionId}"),
                ("Location" = String, description = "Rota project-scoped da execução iniciada"),
                ("x-transaction-id" = Option<String>, description = "Eco do x-transaction-id recebido")
            )
        ),
        (
            status = 400,
            description = "Request inválido",
            body = ErrorResponse,
            headers(
                ("x-transaction-id" = Option<String>, description = "Eco do x-transaction-id recebido")
            )
        ),
        (
            status = 503,
            description = "Sem runners disponíveis",
            body = ErrorResponse,
            headers(
                ("x-transaction-id" = Option<String>, description = "Eco do x-transaction-id recebido")
            )
        )
    )
)]
pub async fn run_e2e_test_for_project(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
    payload: Result<Json<ProjectE2eTestRequest>, JsonRejection>,
) -> Response {
    let Json(payload) = match payload {
        Ok(payload) => payload,
        Err(rejection) => return bad_request_response(rejection),
    };

    let (pipeline, pipeline_index) = match (payload.pipeline_id.clone(), payload.pipeline) {
        (Some(pipeline_id), _) if !pipeline_id.trim().is_empty() => {
            match load_project_pipeline_for_execution(&state.db, &project_id, &pipeline_id).await {
                Ok(Some((pipeline, position))) => (pipeline, Some(position)),
                Ok(None) => {
                    return bad_request_message_response("pipelineId not found for project");
                }
                Err(err) => {
                    return internal_error_response(format!(
                        "failed to load pipeline for execution: {err}"
                    ));
                }
            }
        }
        (_, Some(pipeline)) => (pipeline, payload.pipeline_index),
        _ => return bad_request_message_response("pipelineId is required"),
    };

    let forwarded = E2eTestRequest {
        pipeline,
        selected_base_url_key: payload.selected_base_url_key,
        selected_env_group_slug: payload.selected_env_group_slug,
        project_id: Some(project_id.clone()),
        pipeline_index,
        start_step_id: None,
        prior_results: Default::default(),
        specs: payload.specs,
        env_groups: payload.env_groups,
    };
    run_e2e_test_internal(State(state), project_id, headers, Ok(Json(forwarded))).await
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{projectId}/tests/e2e/rerun-from-step",
    params(
        ("projectId" = String, Path, description = "ID do projeto"),
        ("x-transaction-id" = Option<String>, Header, description = "ID de transação para rastreamento; será propagado para os runners e ecoado no response")
    ),
    request_body = ProjectE2eRerunFromStepRequest,
    responses(
        (
            status = 200,
            description = "Stream SSE para reexecução E2E a partir de um step.",
            content_type = "text/event-stream",
            body = OrchestratorSseEventData,
            headers(
                ("x-execution-id" = String, description = "ID da execução iniciada para reconexão via GET /executions/{executionId}"),
                ("Location" = String, description = "Rota project-scoped da execução iniciada"),
                ("x-transaction-id" = Option<String>, description = "Eco do x-transaction-id recebido")
            )
        ),
        (
            status = 400,
            description = "Request inválido",
            body = ErrorResponse,
            headers(
                ("x-transaction-id" = Option<String>, description = "Eco do x-transaction-id recebido")
            )
        ),
        (
            status = 503,
            description = "Sem runners disponíveis",
            body = ErrorResponse,
            headers(
                ("x-transaction-id" = Option<String>, description = "Eco do x-transaction-id recebido")
            )
        )
    )
)]
pub async fn run_e2e_rerun_from_step_for_project(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
    payload: Result<Json<ProjectE2eRerunFromStepRequest>, JsonRejection>,
) -> Response {
    let Json(payload) = match payload {
        Ok(payload) => payload,
        Err(rejection) => return bad_request_response(rejection),
    };

    let (pipeline, pipeline_index) = match (payload.pipeline_id.clone(), payload.pipeline) {
        (Some(pipeline_id), _) if !pipeline_id.trim().is_empty() => {
            match load_project_pipeline_for_execution(&state.db, &project_id, &pipeline_id).await {
                Ok(Some((pipeline, position))) => (pipeline, Some(position)),
                Ok(None) => {
                    return bad_request_message_response("pipelineId not found for project");
                }
                Err(err) => {
                    return internal_error_response(format!(
                        "failed to load pipeline for execution: {err}"
                    ));
                }
            }
        }
        (_, Some(pipeline)) => (pipeline, payload.pipeline_index),
        _ => return bad_request_message_response("pipelineId is required"),
    };

    let forwarded = E2eTestRequest {
        pipeline,
        selected_base_url_key: payload.selected_base_url_key,
        selected_env_group_slug: payload.selected_env_group_slug,
        project_id: Some(project_id.clone()),
        pipeline_index,
        start_step_id: Some(payload.start_step_id),
        prior_results: payload.prior_results,
        specs: payload.specs,
        env_groups: payload.env_groups,
    };
    run_e2e_test_internal(State(state), project_id, headers, Ok(Json(forwarded))).await
}

fn response_with_execution_headers(
    project_id: String,
    execution_id: &str,
    mut response: Response,
) -> Response {
    let location = format!("/api/v1/projects/{project_id}/executions/{execution_id}");
    response.headers_mut().insert(
        "x-execution-id",
        HeaderValue::from_str(execution_id).unwrap_or_else(|_| HeaderValue::from_static("")),
    );
    response.headers_mut().insert(
        header::LOCATION,
        HeaderValue::from_str(&location).unwrap_or_else(|_| HeaderValue::from_static("")),
    );
    response
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::convert::Infallible;
    use std::sync::Arc;

    use axum::body::{Body, Bytes};
    use axum::extract::State;
    use axum::http::{Method, Request, StatusCode, header};
    use axum::response::{IntoResponse, Response};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use previa_runner::{Pipeline, PipelineStep};
    use reqwest::Client;
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use tokio::sync::{Mutex, RwLock, mpsc};
    use tokio_stream::StreamExt;
    use tokio_stream::wrappers::ReceiverStream;
    use tower::ServiceExt;

    use crate::server::build_app;
    use crate::server::db::insert_project_pipeline;
    use crate::server::execution::ExecutionScheduler;
    use crate::server::mcp::models::McpConfig;
    use crate::server::state::AppState;

    #[tokio::test]
    async fn post_e2e_returns_execution_headers_matching_execution_init_event() {
        let (runner_url, _runner_task) = spawn_runner_server().await;
        let app = test_app(runner_url).await;

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/projects/project-1/tests/e2e")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({ "pipelineId": "pipe-1" })).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/event-stream")
        );
        let execution_id = response
            .headers()
            .get("x-execution-id")
            .and_then(|value| value.to_str().ok())
            .expect("execution id header")
            .to_owned();
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .expect("location header")
            .to_owned();
        assert!(location.ends_with(&format!("/executions/{execution_id}")));

        let mut body = response.into_body().into_data_stream();
        let first_chunk = tokio::time::timeout(std::time::Duration::from_secs(2), body.next())
            .await
            .expect("first chunk timeout")
            .expect("first chunk exists")
            .expect("body chunk");
        let payload = String::from_utf8(first_chunk.to_vec()).unwrap();
        assert!(payload.contains("event: execution:init"));
        assert!(payload.contains(&format!("\"executionId\":\"{execution_id}\"")));
    }

    #[tokio::test]
    async fn post_e2e_rerun_from_step_forwards_start_step_and_prior_results() {
        let received = Arc::new(Mutex::new(None));
        let (runner_url, _runner_task) = spawn_rerun_runner_server(Arc::clone(&received)).await;
        let app = test_app(runner_url).await;

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/projects/project-1/tests/e2e/rerun-from-step")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "pipelineId": "pipe-1",
                            "startStepId": "step-1",
                            "priorResults": {
                                "setup": {
                                    "stepId": "setup",
                                    "status": "success",
                                    "response": {
                                        "status": 200,
                                        "statusText": "OK",
                                        "headers": {},
                                        "body": { "token": "abc123" }
                                    }
                                }
                            }
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let mut body = response.into_body().into_data_stream();
        let first_chunk = tokio::time::timeout(std::time::Duration::from_secs(2), body.next())
            .await
            .expect("first chunk timeout")
            .expect("first chunk exists")
            .expect("body chunk");
        let payload = String::from_utf8(first_chunk.to_vec()).unwrap();
        assert!(payload.contains("event: execution:init"));

        let mut forwarded = None;
        for _ in 0..20 {
            forwarded = received.lock().await.clone();
            if forwarded.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        let forwarded = forwarded.expect("forwarded payload");
        assert_eq!(forwarded["startStepId"], "step-1");
        assert_eq!(
            forwarded["priorResults"]["setup"]["response"]["body"]["token"],
            "abc123"
        );
    }

    async fn test_app(runner_url: String) -> Router {
        let db = crate::server::db::DbPool::connect("sqlite::memory:", 1)
            .await
            .expect("sqlite memory db");
        sqlx::migrate!("./migrations/sqlite")
            .run(db.pool())
            .await
            .expect("migrations");

        sqlx::query(
            "INSERT INTO projects (
                id, name, description, created_at, updated_at, created_at_ms, updated_at_ms, spec_json, execution_backend_url
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("project-1")
        .bind("Project")
        .bind(Option::<String>::None)
        .bind("2026-03-13T00:00:00.000Z")
        .bind("2026-03-13T00:00:00.000Z")
        .bind(0_i64)
        .bind(0_i64)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .execute(&db)
        .await
        .expect("insert project");

        insert_project_pipeline(&db, "project-1", pipeline("pipe-1"))
            .await
            .expect("insert pipeline");
        crate::server::db::seed_env_runner_records(&db, &[runner_url])
            .await
            .expect("seed runner");

        let state = AppState {
            client: Client::new(),
            db,
            context_name: "default".to_owned(),
            runner_auth_key: None,
            rps_per_node: 1000,
            scheduler: ExecutionScheduler::new(Default::default()),
            executions: Arc::new(RwLock::new(HashMap::new())),
            e2e_queues: Arc::new(RwLock::new(HashMap::new())),
            mcp_sessions: Arc::new(RwLock::new(HashMap::new())),
        };

        build_app(
            state,
            &McpConfig {
                enabled: false,
                path: "/mcp".to_owned(),
            },
        )
    }

    fn pipeline(id: &str) -> Pipeline {
        Pipeline {
            id: Some(id.to_owned()),
            name: "Pipeline".to_owned(),
            description: None,
            steps: vec![PipelineStep {
                id: "step-1".to_owned(),
                name: "step-1".to_owned(),
                description: None,
                method: "GET".to_owned(),
                url: "https://example.com".to_owned(),
                headers: HashMap::new(),
                body: None,
                operation_id: None,
                delay: None,
                retry: None,
                asserts: Vec::new(),
            }],
        }
    }

    async fn spawn_runner_server() -> (String, tokio::task::JoinHandle<()>) {
        async fn health() -> impl IntoResponse {
            Json(json!({ "status": "ok" }))
        }

        async fn e2e(State(()): State<()>, Json(_payload): Json<Value>) -> Response {
            let (tx, rx) = mpsc::channel::<Result<Bytes, Infallible>>(8);
            tokio::spawn(async move {
                let _ = tx
                    .send(Ok(Bytes::from(
                        "event: execution:init\ndata: {\"executionId\":\"runner-exec\",\"status\":\"running\"}\n\n",
                    )))
                    .await;
            });

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/event-stream")
                .body(Body::from_stream(ReceiverStream::new(rx)))
                .unwrap()
        }

        let app = Router::new()
            .route("/health", get(health))
            .route("/api/v1/tests/e2e", post(e2e))
            .with_state(());

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("runner server");
        });
        (format!("http://{}", addr), task)
    }

    async fn spawn_rerun_runner_server(
        received: Arc<Mutex<Option<Value>>>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        async fn health() -> impl IntoResponse {
            Json(json!({ "status": "ok" }))
        }

        async fn e2e_rerun(
            State(received): State<Arc<Mutex<Option<Value>>>>,
            Json(payload): Json<Value>,
        ) -> Response {
            *received.lock().await = Some(payload);
            let (tx, rx) = mpsc::channel::<Result<Bytes, Infallible>>(8);
            tokio::spawn(async move {
                let _ = tx
                    .send(Ok(Bytes::from(
                        "event: execution:init\ndata: {\"executionId\":\"runner-rerun\",\"status\":\"running\"}\n\n",
                    )))
                    .await;
            });

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/event-stream")
                .body(Body::from_stream(ReceiverStream::new(rx)))
                .unwrap()
        }

        let app = Router::new()
            .route("/health", get(health))
            .route("/api/v1/tests/e2e/rerun-from-step", post(e2e_rerun))
            .with_state(received);

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("runner server");
        });
        (format!("http://{}", addr), task)
    }
}
