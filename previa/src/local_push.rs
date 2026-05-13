use anyhow::{Context, Result, bail};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use urlencoding::encode;

use crate::auth::{apply_optional_bearer, auth_path_for_context, auth_path_for_url};
use crate::cli::LocalPushArgs;
use crate::paths::PreviaPaths;

const PROJECT_EXPORT_FORMAT: &str = "previa.project.export.v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalPushOutcome {
    pub project_id: String,
    pub project_name: String,
    pub remote_project_replaced: Option<String>,
    pub pipelines_imported: usize,
    pub specs_imported: usize,
    pub e2e_history_imported: usize,
    pub load_history_imported: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectRecord {
    id: String,
    name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectExportEnvelope {
    format: String,
    exported_at: String,
    history_included: bool,
    project: ProjectExportProject,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectExportProject {
    id: String,
    name: String,
    description: Option<String>,
    created_at: String,
    updated_at: String,
    spec: Option<Value>,
    pipelines: Vec<Value>,
    specs: Vec<Value>,
    history: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectImportResponse {
    project_id: String,
    pipelines_imported: usize,
    specs_imported: usize,
    e2e_history_imported: usize,
    load_history_imported: usize,
}

#[derive(Debug, Deserialize)]
struct ApiErrorResponse {
    message: Option<String>,
}

pub async fn push_project(
    paths: &PreviaPaths,
    http: &Client,
    local_base_url: &str,
    args: &LocalPushArgs,
) -> Result<LocalPushOutcome> {
    let remote_base_url = normalize_remote_url(&args.to)?;
    let local_auth = auth_path_for_context(paths, &args.context)?;
    let remote_auth = auth_path_for_url(paths, &remote_base_url);
    push_project_between(
        http,
        local_base_url,
        Some(&local_auth),
        &remote_base_url,
        Some(&remote_auth),
        args,
    )
    .await
}

async fn push_project_between(
    http: &Client,
    local_base_url: &str,
    local_auth_path: Option<&std::path::PathBuf>,
    remote_base_url: &str,
    remote_auth_path: Option<&std::path::PathBuf>,
    args: &LocalPushArgs,
) -> Result<LocalPushOutcome> {
    let local_base_url = trim_base_url(local_base_url)?;
    let remote_base_url = trim_base_url(remote_base_url)?;
    let local_project = resolve_project(
        http,
        local_base_url,
        local_auth_path,
        &args.project,
        "local",
    )
    .await?;
    let envelope = export_project(
        http,
        local_base_url,
        local_auth_path,
        &local_project.id,
        args.include_history,
        "local",
    )
    .await?;

    if envelope.format != PROJECT_EXPORT_FORMAT {
        bail!(
            "local export returned unsupported format '{}'",
            envelope.format
        );
    }

    let remote_existing = if let Some(remote_project_id) = args.remote_project_id.as_deref() {
        load_project_by_id(
            http,
            remote_base_url,
            remote_auth_path,
            remote_project_id,
            "remote",
        )
        .await?
    } else {
        resolve_existing_remote_project(http, remote_base_url, remote_auth_path, &envelope.project)
            .await?
    };

    if let Some(remote_project) = remote_existing.as_ref() {
        if !args.overwrite {
            bail!(
                "project '{}' ({}) already exists on remote; rerun with --overwrite to replace it",
                remote_project.name,
                remote_project.id
            );
        }

        delete_project(http, remote_base_url, remote_auth_path, &remote_project.id).await?;
    }

    let imported = import_project(
        http,
        remote_base_url,
        remote_auth_path,
        &envelope,
        args.include_history,
    )
    .await?;

    Ok(LocalPushOutcome {
        project_id: imported.project_id,
        project_name: envelope.project.name,
        remote_project_replaced: remote_existing.map(|project| project.id),
        pipelines_imported: imported.pipelines_imported,
        specs_imported: imported.specs_imported,
        e2e_history_imported: imported.e2e_history_imported,
        load_history_imported: imported.load_history_imported,
    })
}

fn normalize_remote_url(raw: &str) -> Result<String> {
    let value = raw.trim();
    if value.is_empty() {
        bail!("--to is required");
    }
    if !value.starts_with("http://") && !value.starts_with("https://") {
        bail!("--to must start with http:// or https://");
    }
    Ok(value.trim_end_matches('/').to_owned())
}

fn trim_base_url(raw: &str) -> Result<&str> {
    let value = raw.trim().trim_end_matches('/');
    if value.is_empty() {
        bail!("base URL cannot be empty");
    }
    Ok(value)
}

async fn resolve_existing_remote_project(
    http: &Client,
    remote_base_url: &str,
    auth_path: Option<&std::path::PathBuf>,
    project: &ProjectExportProject,
) -> Result<Option<ProjectRecord>> {
    if let Some(project) =
        load_project_by_id(http, remote_base_url, auth_path, &project.id, "remote").await?
    {
        return Ok(Some(project));
    }

    let matches = list_all_projects(http, remote_base_url, auth_path, "remote")
        .await?
        .into_iter()
        .filter(|remote| remote.name == project.name)
        .collect::<Vec<_>>();

    match matches.len() {
        0 => Ok(None),
        1 => Ok(matches.into_iter().next()),
        _ => {
            let ids = matches
                .iter()
                .map(|project| project.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "remote project name '{}' is ambiguous; matched project ids: {}. Retry with --remote-project-id <id>",
                project.name,
                ids
            );
        }
    }
}

async fn resolve_project(
    http: &Client,
    api_base: &str,
    auth_path: Option<&std::path::PathBuf>,
    selector: &str,
    label: &str,
) -> Result<ProjectRecord> {
    if let Some(project) = load_project_by_id(http, api_base, auth_path, selector, label).await? {
        return Ok(project);
    }

    let matches = list_all_projects(http, api_base, auth_path, label)
        .await?
        .into_iter()
        .filter(|project| project.name == selector)
        .collect::<Vec<_>>();

    match matches.len() {
        0 => bail!("{label} project '{selector}' not found"),
        1 => Ok(matches.into_iter().next().expect("single project match")),
        _ => {
            let ids = matches
                .iter()
                .map(|project| project.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "{label} project name '{}' is ambiguous; matched project ids: {}. Retry with --project <id>",
                selector,
                ids
            );
        }
    }
}

async fn load_project_by_id(
    http: &Client,
    api_base: &str,
    auth_path: Option<&std::path::PathBuf>,
    selector: &str,
    label: &str,
) -> Result<Option<ProjectRecord>> {
    let url = format!("{}/api/v1/projects/{}", api_base, encode(selector));
    let request = http.get(&url);
    let request = match auth_path {
        Some(path) => apply_optional_bearer(request, path)?,
        None => request,
    };
    let response = request
        .send()
        .await
        .with_context(|| format!("failed to query {label} project API at '{url}'"))?;

    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        let status = response.status();
        let message = decode_error_message(response).await;
        bail!(
            "failed to query {label} project API: {} ({status})",
            message
        );
    }

    response
        .json::<ProjectRecord>()
        .await
        .with_context(|| format!("failed to decode {label} project response from '{url}'"))
        .map(Some)
}

async fn list_all_projects(
    http: &Client,
    api_base: &str,
    auth_path: Option<&std::path::PathBuf>,
    label: &str,
) -> Result<Vec<ProjectRecord>> {
    let mut offset = 0u32;
    let limit = 500u32;
    let mut projects = Vec::new();

    loop {
        let url = format!("{api_base}/api/v1/projects?limit={limit}&offset={offset}");
        let request = http.get(&url);
        let request = match auth_path {
            Some(path) => apply_optional_bearer(request, path)?,
            None => request,
        };
        let response = request
            .send()
            .await
            .with_context(|| format!("failed to query {label} projects API at '{url}'"))?;

        if !response.status().is_success() {
            let status = response.status();
            let message = decode_error_message(response).await;
            bail!(
                "failed to query {label} projects API: {} ({status})",
                message
            );
        }

        let batch = response
            .json::<Vec<ProjectRecord>>()
            .await
            .with_context(|| format!("failed to decode {label} projects response from '{url}'"))?;
        let batch_len = batch.len();
        projects.extend(batch);
        if batch_len < limit as usize {
            break;
        }
        offset += limit;
    }

    Ok(projects)
}

async fn export_project(
    http: &Client,
    api_base: &str,
    auth_path: Option<&std::path::PathBuf>,
    project_id: &str,
    include_history: bool,
    label: &str,
) -> Result<ProjectExportEnvelope> {
    let url = format!(
        "{}/api/v1/projects/{}/export?includeHistory={}",
        api_base,
        encode(project_id),
        include_history
    );
    let request = http.get(&url);
    let request = match auth_path {
        Some(path) => apply_optional_bearer(request, path)?,
        None => request,
    };
    let response = request
        .send()
        .await
        .with_context(|| format!("failed to export {label} project at '{url}'"))?;

    if !response.status().is_success() {
        let status = response.status();
        let message = decode_error_message(response).await;
        bail!("failed to export {label} project: {} ({status})", message);
    }

    response
        .json::<ProjectExportEnvelope>()
        .await
        .with_context(|| format!("failed to decode {label} project export from '{url}'"))
}

async fn delete_project(
    http: &Client,
    api_base: &str,
    auth_path: Option<&std::path::PathBuf>,
    project_id: &str,
) -> Result<()> {
    let url = format!("{}/api/v1/projects/{}", api_base, encode(project_id));
    let request = http.delete(&url);
    let request = match auth_path {
        Some(path) => apply_optional_bearer(request, path)?,
        None => request,
    };
    let response = request
        .send()
        .await
        .with_context(|| format!("failed to delete remote project at '{url}'"))?;

    if !response.status().is_success() {
        let status = response.status();
        let message = decode_error_message(response).await;
        bail!("failed to delete remote project: {} ({status})", message);
    }

    Ok(())
}

async fn import_project(
    http: &Client,
    api_base: &str,
    auth_path: Option<&std::path::PathBuf>,
    envelope: &ProjectExportEnvelope,
    include_history: bool,
) -> Result<ProjectImportResponse> {
    let url = format!("{api_base}/api/v1/projects/import?includeHistory={include_history}");
    let request = http.post(&url);
    let request = match auth_path {
        Some(path) => apply_optional_bearer(request, path)?,
        None => request,
    };
    let response = request
        .header("content-type", "application/json")
        .json(envelope)
        .send()
        .await
        .with_context(|| format!("failed to import project into remote at '{url}'"))?;

    if !response.status().is_success() {
        let status = response.status();
        let message = decode_error_message(response).await;
        bail!(
            "failed to import project into remote: {} ({status})",
            message
        );
    }

    response
        .json::<ProjectImportResponse>()
        .await
        .with_context(|| format!("failed to decode remote project import response from '{url}'"))
}

async fn decode_error_message(response: reqwest::Response) -> String {
    let body = response.text().await.unwrap_or_default();
    serde_json::from_str::<ApiErrorResponse>(&body)
        .ok()
        .and_then(|payload| payload.message)
        .filter(|message| !message.trim().is_empty())
        .unwrap_or_else(|| body.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Path, Query, State};
    use axum::http::StatusCode;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use serde::Deserialize;
    use serde_json::{Value, json};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;

    #[derive(Clone, Default)]
    struct MockState {
        projects: Arc<Mutex<HashMap<String, Value>>>,
        deleted: Arc<Mutex<Vec<String>>>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct IncludeHistoryQuery {
        include_history: Option<bool>,
    }

    async fn spawn_mock_app(state: MockState) -> String {
        let app = Router::new()
            .route("/api/v1/projects", get(list_projects))
            .route(
                "/api/v1/projects/{project_id}",
                get(get_project).delete(delete_project_mock),
            )
            .route(
                "/api/v1/projects/{project_id}/export",
                get(export_project_mock),
            )
            .route("/api/v1/projects/import", post(import_project_mock))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve mock");
        });
        format!("http://{addr}")
    }

    async fn list_projects(State(state): State<MockState>) -> Json<Vec<Value>> {
        let projects = state.projects.lock().await;
        Json(
            projects
                .values()
                .map(|project| {
                    json!({
                        "id": project["project"]["id"],
                        "name": project["project"]["name"],
                        "description": project["project"]["description"],
                        "createdAt": project["project"]["createdAt"],
                        "updatedAt": project["project"]["updatedAt"],
                    })
                })
                .collect(),
        )
    }

    async fn get_project(
        State(state): State<MockState>,
        Path(project_id): Path<String>,
    ) -> Result<Json<Value>, StatusCode> {
        let projects = state.projects.lock().await;
        let Some(project) = projects.get(&project_id) else {
            return Err(StatusCode::NOT_FOUND);
        };

        Ok(Json(json!({
            "id": project["project"]["id"],
            "name": project["project"]["name"],
            "description": project["project"]["description"],
            "createdAt": project["project"]["createdAt"],
            "updatedAt": project["project"]["updatedAt"],
        })))
    }

    async fn export_project_mock(
        State(state): State<MockState>,
        Path(project_id): Path<String>,
        Query(query): Query<IncludeHistoryQuery>,
    ) -> Result<Json<Value>, StatusCode> {
        let projects = state.projects.lock().await;
        let Some(project) = projects.get(&project_id) else {
            return Err(StatusCode::NOT_FOUND);
        };
        let mut envelope = project.clone();
        envelope["historyIncluded"] = json!(query.include_history.unwrap_or(true));
        Ok(Json(envelope))
    }

    async fn delete_project_mock(
        State(state): State<MockState>,
        Path(project_id): Path<String>,
    ) -> StatusCode {
        state.projects.lock().await.remove(&project_id);
        state.deleted.lock().await.push(project_id);
        StatusCode::NO_CONTENT
    }

    async fn import_project_mock(
        State(state): State<MockState>,
        Query(query): Query<IncludeHistoryQuery>,
        Json(mut envelope): Json<Value>,
    ) -> Result<(StatusCode, Json<Value>), StatusCode> {
        let project_id = envelope["project"]["id"]
            .as_str()
            .expect("project id")
            .to_owned();
        let mut projects = state.projects.lock().await;
        if projects.contains_key(&project_id) {
            return Err(StatusCode::CONFLICT);
        }
        envelope["historyIncluded"] = json!(query.include_history.unwrap_or(true));
        let pipelines_imported = envelope["project"]["pipelines"]
            .as_array()
            .map_or(0, Vec::len);
        let specs_imported = envelope["project"]["specs"].as_array().map_or(0, Vec::len);
        projects.insert(project_id.clone(), envelope);
        Ok((
            StatusCode::CREATED,
            Json(json!({
                "projectId": project_id,
                "includeHistory": query.include_history.unwrap_or(true),
                "pipelinesImported": pipelines_imported,
                "specsImported": specs_imported,
                "e2eHistoryImported": 0,
                "loadHistoryImported": 0
            })),
        ))
    }

    fn project_envelope(id: &str, name: &str) -> Value {
        json!({
            "format": PROJECT_EXPORT_FORMAT,
            "exportedAt": "2026-04-29T00:00:00Z",
            "historyIncluded": false,
            "project": {
                "id": id,
                "name": name,
                "description": null,
                "createdAt": "2026-04-29T00:00:00Z",
                "updatedAt": "2026-04-29T00:00:00Z",
                "spec": null,
                "pipelines": [{ "id": "pipe-1", "name": "Smoke", "description": null, "steps": [] }],
                "specs": [],
                "history": { "e2e": [], "load": [] }
            }
        })
    }

    fn push_args(overwrite: bool) -> LocalPushArgs {
        LocalPushArgs {
            context: "default".to_owned(),
            project: "Local App".to_owned(),
            to: "http://unused.test".to_owned(),
            remote_project_id: None,
            overwrite,
            include_history: false,
        }
    }

    #[tokio::test]
    async fn push_creates_remote_project_when_missing() {
        let local = MockState::default();
        local.projects.lock().await.insert(
            "local-1".to_owned(),
            project_envelope("local-1", "Local App"),
        );
        let remote = MockState::default();
        let local_url = spawn_mock_app(local).await;
        let remote_url = spawn_mock_app(remote.clone()).await;
        let http = Client::new();
        let args = LocalPushArgs {
            to: remote_url.clone(),
            ..push_args(false)
        };

        let outcome = push_project_between(&http, &local_url, None, &remote_url, None, &args)
            .await
            .expect("push succeeds");

        assert_eq!(outcome.project_id, "local-1");
        assert_eq!(outcome.remote_project_replaced, None);
        assert_eq!(outcome.pipelines_imported, 1);
        assert!(remote.projects.lock().await.contains_key("local-1"));
    }

    #[tokio::test]
    async fn push_requires_overwrite_when_remote_project_exists() {
        let local = MockState::default();
        local.projects.lock().await.insert(
            "local-1".to_owned(),
            project_envelope("local-1", "Local App"),
        );
        let remote = MockState::default();
        remote.projects.lock().await.insert(
            "local-1".to_owned(),
            project_envelope("local-1", "Local App"),
        );
        let local_url = spawn_mock_app(local).await;
        let remote_url = spawn_mock_app(remote).await;
        let http = Client::new();
        let args = LocalPushArgs {
            to: remote_url.clone(),
            ..push_args(false)
        };

        let err = push_project_between(&http, &local_url, None, &remote_url, None, &args)
            .await
            .expect_err("push should fail");

        assert!(err.to_string().contains("--overwrite"));
    }

    #[tokio::test]
    async fn push_overwrite_deletes_existing_remote_before_import() {
        let local = MockState::default();
        local.projects.lock().await.insert(
            "local-1".to_owned(),
            project_envelope("local-1", "Local App"),
        );
        let remote = MockState::default();
        remote.projects.lock().await.insert(
            "remote-1".to_owned(),
            project_envelope("remote-1", "Local App"),
        );
        let local_url = spawn_mock_app(local).await;
        let remote_url = spawn_mock_app(remote.clone()).await;
        let http = Client::new();
        let args = LocalPushArgs {
            to: remote_url.clone(),
            ..push_args(true)
        };

        let outcome = push_project_between(&http, &local_url, None, &remote_url, None, &args)
            .await
            .expect("push succeeds");

        assert_eq!(outcome.remote_project_replaced.as_deref(), Some("remote-1"));
        assert_eq!(remote.deleted.lock().await.as_slice(), ["remote-1"]);
        let projects = remote.projects.lock().await;
        assert!(projects.contains_key("local-1"));
        assert!(!projects.contains_key("remote-1"));
    }
}
