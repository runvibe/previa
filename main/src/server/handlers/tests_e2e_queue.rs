use axum::Json;
use axum::extract::Path;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::server::errors::bad_request_response;
use crate::server::execution::e2e_queue::{
    QueueError, cancel_e2e_queue, create_e2e_queue, get_current_e2e_queue_response,
    get_e2e_queue_response, queue_error_response,
};
use crate::server::models::{
    E2eQueueRecord, ErrorResponse, OrchestratorSseEventData, ProjectE2eQueueRequest,
};
use crate::server::state::AppState;

#[utoipa::path(
    get,
    path = "/api/v1/projects/{projectId}/tests/e2e/queue",
    params(
        ("projectId" = String, Path, description = "ID do projeto")
    ),
    responses(
        (status = 200, description = "Snapshot da fila E2E ativa do projeto.", body = E2eQueueRecord),
        (status = 404, description = "Nenhuma fila ativa encontrada", body = ErrorResponse),
        (status = 500, description = "Erro ao consultar fila ativa", body = ErrorResponse)
    )
)]
pub async fn get_current_e2e_queue_for_project(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
) -> Response {
    match get_current_e2e_queue_response(state, project_id).await {
        Ok(response) => response,
        Err(err) => queue_error_response(err),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{projectId}/tests/e2e/queue",
    params(
        ("projectId" = String, Path, description = "ID do projeto")
    ),
    request_body = ProjectE2eQueueRequest,
    responses(
        (status = 202, description = "Fila E2E criada", body = E2eQueueRecord),
        (status = 400, description = "Request inválido", body = ErrorResponse),
        (status = 500, description = "Erro ao criar fila", body = ErrorResponse)
    )
)]
pub async fn create_e2e_queue_for_project(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    payload: Result<Json<ProjectE2eQueueRequest>, JsonRejection>,
) -> Response {
    let Json(payload) = match payload {
        Ok(payload) => payload,
        Err(rejection) => return bad_request_response(rejection),
    };

    match create_e2e_queue(state, project_id.clone(), payload).await {
        Ok(snapshot) => {
            let location = format!(
                "/api/v1/projects/{project_id}/tests/e2e/queue/{}",
                snapshot.id
            );
            let mut response = (StatusCode::ACCEPTED, Json(snapshot.clone())).into_response();
            response.headers_mut().insert(
                header::LOCATION,
                HeaderValue::from_str(&location).unwrap_or_else(|_| HeaderValue::from_static("")),
            );
            response.headers_mut().insert(
                "x-queue-id",
                HeaderValue::from_str(&snapshot.id)
                    .unwrap_or_else(|_| HeaderValue::from_static("")),
            );
            response
        }
        Err(err) => queue_error_response(err),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{projectId}/tests/e2e/queue/{queueId}",
    params(
        ("projectId" = String, Path, description = "ID do projeto"),
        ("queueId" = String, Path, description = "ID da fila E2E")
    ),
    responses(
        (
            status = 200,
            description = "SSE quando a fila está ativa, JSON quando está finalizada.",
            content_type = "text/event-stream",
            body = OrchestratorSseEventData
        ),
        (
            status = 200,
            description = "Snapshot JSON da fila finalizada.",
            body = E2eQueueRecord
        ),
        (status = 404, description = "Fila não encontrada", body = ErrorResponse),
        (status = 500, description = "Erro ao consultar fila", body = ErrorResponse)
    )
)]
pub async fn get_e2e_queue_for_project(
    State(state): State<AppState>,
    Path((project_id, queue_id)): Path<(String, String)>,
) -> Response {
    match get_e2e_queue_response(state, project_id, queue_id).await {
        Ok(response) => response,
        Err(err) => queue_error_response(err),
    }
}

#[utoipa::path(
    delete,
    path = "/api/v1/projects/{projectId}/tests/e2e/queue/{queueId}",
    params(
        ("projectId" = String, Path, description = "ID do projeto"),
        ("queueId" = String, Path, description = "ID da fila E2E")
    ),
    responses(
        (status = 204, description = "Fila cancelada"),
        (status = 404, description = "Fila não encontrada", body = ErrorResponse),
        (status = 500, description = "Erro ao cancelar fila", body = ErrorResponse)
    )
)]
pub async fn delete_e2e_queue_for_project(
    State(state): State<AppState>,
    Path((project_id, queue_id)): Path<(String, String)>,
) -> Response {
    match cancel_e2e_queue(state, project_id, queue_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(QueueError::BadRequest(message)) => {
            queue_error_response(QueueError::BadRequest(message))
        }
        Err(err) => queue_error_response(err),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::convert::Infallible;
    use std::sync::Arc;
    use std::time::Duration;

    use axum::Router;
    use axum::body::{Body, Bytes, to_bytes};
    use axum::extract::State;
    use axum::http::{Method, Request, StatusCode, header};
    use axum::response::Response;
    use axum::routing::{get, post};
    use previa_runner::{Pipeline, PipelineStep};
    use reqwest::Client;
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use tokio::sync::{RwLock, mpsc};
    use tokio_stream::StreamExt;
    use tokio_stream::wrappers::ReceiverStream;
    use tower::ServiceExt;

    use crate::server::build_app;
    use crate::server::db::insert_project_pipeline;
    use crate::server::execution::ExecutionScheduler;
    use crate::server::mcp::models::McpConfig;
    use crate::server::state::AppState;

    use super::*;

    #[tokio::test]
    async fn post_queue_returns_headers_and_get_active_returns_sse() {
        let (runner_url, _runner_task) = spawn_runner_server().await;
        let app = test_app(
            vec![runner_url],
            "project-1",
            vec![pipeline("slow"), pipeline("ok")],
        )
        .await;

        let create = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/projects/project-1/tests/e2e/queue")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({ "pipelineIds": ["slow", "ok"] })).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(create.status(), StatusCode::ACCEPTED);
        let (queue_id, location) = queue_id_and_location_from_response(create).await;
        assert!(location.ends_with(&queue_id));

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!(
                        "/api/v1/projects/project-1/tests/e2e/queue/{queue_id}"
                    ))
                    .body(Body::empty())
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

        let mut body = response.into_body().into_data_stream();
        let first_chunk = tokio::time::timeout(Duration::from_secs(2), body.next())
            .await
            .expect("first chunk timeout")
            .expect("first chunk exists")
            .expect("body chunk");
        let payload = String::from_utf8(first_chunk.to_vec()).unwrap();
        assert!(payload.contains("event: queue:update"));
        assert!(
            payload.contains("\"status\":\"pending\"")
                || payload.contains("\"status\":\"running\"")
        );
    }

    #[tokio::test]
    async fn get_current_queue_returns_active_snapshot() {
        let (runner_url, _runner_task) = spawn_runner_server().await;
        let app = test_app(
            vec![runner_url],
            "project-1",
            vec![pipeline("slow"), pipeline("ok")],
        )
        .await;

        let create = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/projects/project-1/tests/e2e/queue")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({ "pipelineIds": ["slow", "ok"] })).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        let (queue_id, _) = queue_id_and_location_from_response(create).await;
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/projects/project-1/tests/e2e/queue")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let snapshot = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(snapshot["id"], json!(queue_id));
        assert!(snapshot["status"] == json!("pending") || snapshot["status"] == json!("running"));
    }

    #[tokio::test]
    async fn get_current_queue_returns_not_found_without_active_queue() {
        let app = test_app(Vec::new(), "project-1", vec![pipeline("ok")]).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/projects/project-1/tests/e2e/queue")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn queue_failure_marks_remaining_items_cancelled() {
        let (runner_url, _runner_task) = spawn_runner_server().await;
        let app = test_app(
            vec![runner_url],
            "project-1",
            vec![pipeline("ok"), pipeline("fail"), pipeline("after")],
        )
        .await;

        let create = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/projects/project-1/tests/e2e/queue")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({ "pipelineIds": ["ok", "fail", "after"] }))
                            .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let (queue_id, _) = queue_id_and_location_from_response(create).await;

        let snapshot = wait_for_terminal_queue(&app, "project-1", &queue_id).await;
        assert_eq!(snapshot["status"], json!("failed"));
        assert_eq!(snapshot["pipelines"][0]["status"], json!("completed"));
        assert_eq!(snapshot["pipelines"][1]["status"], json!("failed"));
        assert_eq!(snapshot["pipelines"][2]["status"], json!("cancelled"));
    }

    #[tokio::test]
    async fn queue_waits_for_existing_project_execution_before_starting() {
        let (runner_url, _runner_task) = spawn_runner_server().await;
        let app = test_app(
            vec![runner_url],
            "project-1",
            vec![pipeline("slow"), pipeline("ok")],
        )
        .await;

        let _load_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/projects/project-1/tests/load")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "pipelineId": "slow",
                            "config": {
                                "totalRequests": 1,
                                "concurrency": 1,
                                "rampUpSeconds": 0.0
                            }
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;

        let create = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/projects/project-1/tests/e2e/queue")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({ "pipelineIds": ["ok"] })).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let (queue_id, _) = queue_id_and_location_from_response(create).await;

        tokio::time::sleep(Duration::from_millis(100)).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/projects/project-1/tests/e2e/queue")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let snapshot = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(snapshot["id"], json!(queue_id));
        assert_eq!(snapshot["status"], json!("pending"));
        assert_eq!(snapshot["pipelines"][0]["status"], json!("pending"));

        let terminal = wait_for_terminal_queue(&app, "project-1", &queue_id).await;
        assert_eq!(terminal["status"], json!("completed"));
        assert_eq!(terminal["pipelines"][0]["status"], json!("completed"));
    }

    #[tokio::test]
    async fn deleting_queue_cancels_running_and_pending_items() {
        let (runner_url, _runner_task) = spawn_runner_server().await;
        let app = test_app(
            vec![runner_url],
            "project-1",
            vec![pipeline("slow"), pipeline("after")],
        )
        .await;

        let create = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/projects/project-1/tests/e2e/queue")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({ "pipelineIds": ["slow", "after"] })).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let (queue_id, _) = queue_id_and_location_from_response(create).await;

        let delete = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri(format!(
                        "/api/v1/projects/project-1/tests/e2e/queue/{queue_id}"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete.status(), StatusCode::NO_CONTENT);

        let snapshot = wait_for_terminal_queue(&app, "project-1", &queue_id).await;
        assert_eq!(snapshot["status"], json!("cancelled"));
        assert_eq!(snapshot["pipelines"][0]["status"], json!("cancelled"));
        assert_eq!(snapshot["pipelines"][1]["status"], json!("cancelled"));
    }

    #[tokio::test]
    async fn creating_new_queue_cancels_existing_queue_for_same_project() {
        let (runner_url, _runner_task) = spawn_runner_server().await;
        let app = test_app(
            vec![runner_url],
            "project-1",
            vec![pipeline("slow"), pipeline("ok")],
        )
        .await;

        let first = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/projects/project-1/tests/e2e/queue")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({ "pipelineIds": ["slow"] })).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let (first_queue_id, _) = queue_id_and_location_from_response(first).await;

        let second = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/projects/project-1/tests/e2e/queue")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({ "pipelineIds": ["ok"] })).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let (second_queue_id, _) = queue_id_and_location_from_response(second).await;

        let first_snapshot = wait_for_terminal_queue(&app, "project-1", &first_queue_id).await;
        let second_snapshot = wait_for_terminal_queue(&app, "project-1", &second_queue_id).await;

        assert_eq!(first_snapshot["status"], json!("cancelled"));
        assert_eq!(second_snapshot["status"], json!("completed"));
        assert_eq!(
            second_snapshot["pipelines"][0]["status"],
            json!("completed")
        );
    }

    async fn queue_id_and_location_from_response(response: Response) -> (String, String) {
        let header_queue_id = response
            .headers()
            .get("x-queue-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
            .unwrap();
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
            .unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let snapshot = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(snapshot["id"], json!(header_queue_id));
        (header_queue_id, location)
    }

    async fn wait_for_terminal_queue(app: &Router, project_id: &str, queue_id: &str) -> Value {
        for _ in 0..50 {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::GET)
                        .uri(format!(
                            "/api/v1/projects/{project_id}/tests/e2e/queue/{queue_id}"
                        ))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            let content_type = response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_owned();
            if content_type.starts_with("application/json") {
                let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
                let value = serde_json::from_slice::<Value>(&body).unwrap();
                if value["status"] != json!("pending") && value["status"] != json!("running") {
                    return value;
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        panic!("queue did not reach terminal state");
    }

    async fn test_app(
        runner_endpoints: Vec<String>,
        project_id: &str,
        pipelines: Vec<Pipeline>,
    ) -> Router {
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
        .bind(project_id)
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

        for pipeline in pipelines {
            insert_project_pipeline(&db, project_id, pipeline)
                .await
                .expect("insert pipeline");
        }
        crate::server::db::seed_env_runner_records(&db, &runner_endpoints)
            .await
            .expect("seed runners");

        let state = AppState {
            client: Client::new(),
            db,
            context_name: "default".to_owned(),
            runner_auth_key: None,
            auth: crate::server::auth::AuthRuntime::anonymous(),
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
            name: id.to_owned(),
            description: None,
            steps: vec![PipelineStep {
                id: "step-1".to_owned(),
                name: "step-1".to_owned(),
                description: None,
                method: "GET".to_owned(),
                url: "http://example.com".to_owned(),
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
        async fn send_event(
            tx: &mpsc::Sender<Result<Bytes, Infallible>>,
            event: &str,
            data: Value,
        ) {
            let chunk = format!(
                "event: {event}\ndata: {}\n\n",
                serde_json::to_string(&data).unwrap()
            );
            let _ = tx.send(Ok(Bytes::from(chunk))).await;
        }

        async fn health() -> &'static str {
            "ok"
        }

        async fn e2e(State(()): State<()>, Json(payload): Json<Value>) -> Response {
            let pipeline_id = payload["pipeline"]["id"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let (tx, rx) = mpsc::channel::<Result<Bytes, Infallible>>(8);

            tokio::spawn(async move {
                send_event(
                    &tx,
                    "execution:init",
                    json!({ "executionId": format!("runner-{pipeline_id}") }),
                )
                .await;

                if pipeline_id == "slow" {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                }

                if pipeline_id == "fail" {
                    send_event(
                        &tx,
                        "step:result",
                        json!({
                            "stepId": "step-1",
                            "status": "error",
                            "message": "forced failure"
                        }),
                    )
                    .await;
                    send_event(
                        &tx,
                        "pipeline:complete",
                        json!({ "totalSteps": 1, "passed": 0, "failed": 1, "totalDuration": 1 }),
                    )
                    .await;
                } else {
                    send_event(
                        &tx,
                        "step:result",
                        json!({
                            "stepId": "step-1",
                            "status": "success",
                            "duration": 1,
                            "assertResults": []
                        }),
                    )
                    .await;
                    if pipeline_id == "slow" {
                        tokio::time::sleep(Duration::from_millis(300)).await;
                    }
                    send_event(
                        &tx,
                        "pipeline:complete",
                        json!({ "totalSteps": 1, "passed": 1, "failed": 0, "totalDuration": 1 }),
                    )
                    .await;
                }
            });

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/event-stream")
                .body(Body::from_stream(ReceiverStream::new(rx)))
                .unwrap()
        }

        async fn load(State(()): State<()>, Json(payload): Json<Value>) -> Response {
            let pipeline_id = payload["pipeline"]["id"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let (tx, rx) = mpsc::channel::<Result<Bytes, Infallible>>(8);

            tokio::spawn(async move {
                send_event(
                    &tx,
                    "execution:init",
                    json!({ "executionId": format!("runner-load-{pipeline_id}") }),
                )
                .await;

                send_event(
                    &tx,
                    "metrics",
                    json!({
                        "sent": 1,
                        "completed": 1,
                        "failed": 0,
                        "requestsPerSecond": 1.0,
                        "avgLatencyMs": 1.0,
                        "durationMs": 1
                    }),
                )
                .await;

                if pipeline_id == "slow" {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                }
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
            .route("/api/v1/tests/load", post(load))
            .with_state(());

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{}", addr), task)
    }
}
