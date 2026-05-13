use std::time::Duration;

use previa_runner::Pipeline;
use reqwest::Method;
use reqwest::header::{CONTENT_TYPE, HeaderName, HeaderValue};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::time::timeout;
use tokio_stream::StreamExt;
use tracing::info;

use crate::server::db::{
    DbPool, delete_pipeline_record, delete_project_spec_record, import_project_bundle,
    insert_project_pipeline, insert_project_spec_record, list_e2e_history_records,
    list_load_history_records, list_project_records, list_project_spec_records,
    load_e2e_history_for_export, load_e2e_history_record_by_id, load_load_history_for_export,
    load_load_history_record_by_id, load_pipelines_for_project, load_project_export,
    load_project_pipeline_record, load_project_record, load_project_spec_record_by_id,
    project_exists, update_project_pipeline, update_project_spec_record, upsert_project_metadata,
    upsert_project_with_pipelines,
};
use crate::server::docs::build_openapi_document;
use crate::server::execution::e2e_queue::{
    QueueError, cancel_e2e_queue, create_e2e_queue, get_current_e2e_queue_snapshot,
    get_e2e_queue_snapshot,
};
use crate::server::execution::forward::parse_sse_block;
use crate::server::execution::{
    StartE2eExecutionError, StartLoadExecutionError, resolve_runtime_specs_for_execution,
    start_e2e_execution, start_load_execution,
};
use crate::server::mcp::models::{
    CompletionCompleteParams, CompletionContext, CompletionReference, CompletionResult,
    CompletionValues, CreateProjectArgs, CreateProjectE2eQueueArgs, CreateProjectPipelineArgs,
    CreateProjectSpecArgs, ExecutionByIdArgs, ExecutionCancelArgs, ExportProjectArgs,
    ImportProjectArgs, InitializeParams, ListProjectsToolArgs, LoggingSetLevelParams, McpPeerInfo,
    McpRequest, McpResponse, McpSession, ProjectByIdArgs, ProjectHistoryToolArgs,
    ProjectPipelineByIdArgs, ProjectQueueByIdArgs, ProjectSpecByIdArgs, ProjectTestByIdArgs,
    PromptDefinition, PromptGetParams, PromptGetResult, PromptMessage, PromptTextContent,
    PromptsListParams, ProxyToolArgs, ResourceContents, ResourceDefinition, ResourceReadParams,
    ResourceTemplateDefinition, ResourceTemplatesListParams, ResourcesListParams,
    RunProjectE2eTestArgs, RunProjectLoadTestArgs, SUPPORTED_PROTOCOL_VERSIONS, ToolCallParams,
    ToolCallResult, ToolDefinition, ToolTextContent, ToolsListParams, UpdateProjectArgs,
    UpdateProjectPipelineArgs, UpdateProjectSpecArgs, ValidateOpenApiToolArgs,
};
use crate::server::models::{
    E2eTestRequest, HistoryOrder, HistoryQuery, LoadTestRequest, OrchestratorInfoResponse,
    ProjectE2eQueueRequest, ProjectExportEnvelope, ProjectListQuery, ProxyRequest,
};
use crate::server::services::pipeline_runtime::build_project_pipeline_record;
use crate::server::state::{AppState, ExecutionKind};
use crate::server::utils::{new_uuid_v7, now_iso};
use crate::server::validation::openapi::validate_openapi_source;
use crate::server::validation::pipelines::{KNOWN_TEMPLATE_HELPERS, validate_pipeline_templates};
use crate::server::validation::specs::{normalize_spec_slug, normalize_spec_urls_with_legacy};

pub(crate) const INVALID_REQUEST: i32 = -32600;
const METHOD_NOT_FOUND: i32 = -32601;
const INVALID_PARAMS: i32 = -32602;
const INTERNAL_ERROR: i32 = -32603;
pub(crate) const INVALID_SESSION: i32 = -32001;

pub enum McpHttpOutcome {
    Response {
        response: McpResponse,
        session_id: Option<String>,
        protocol_version: Option<String>,
    },
    Accepted,
}

pub async fn process_request(
    state: &AppState,
    session_id: Option<&str>,
    protocol_version_header: Option<&str>,
    request: McpRequest,
) -> McpHttpOutcome {
    if request.jsonrpc != crate::server::mcp::models::JSON_RPC_VERSION {
        return McpHttpOutcome::Response {
            response: McpResponse::error(request.id, INVALID_REQUEST, "jsonrpc must be 2.0"),
            session_id: None,
            protocol_version: None,
        };
    }

    let Some(request_id) = request.id.clone() else {
        if request.method == "notifications/initialized" {
            return McpHttpOutcome::Accepted;
        }

        return McpHttpOutcome::Response {
            response: McpResponse::error(None, INVALID_REQUEST, "request id is required"),
            session_id: None,
            protocol_version: None,
        };
    };

    match request.method.as_str() {
        "initialize" => handle_initialize(state, request_id, request.params).await,
        "ping" => McpHttpOutcome::Response {
            response: McpResponse::success(request_id, json!({})),
            session_id: session_id.map(str::to_owned),
            protocol_version: None,
        },
        "tools/list" => {
            let session = match require_session(state, session_id, protocol_version_header).await {
                Ok(session) => session,
                Err(response) => {
                    return McpHttpOutcome::Response {
                        response,
                        session_id: None,
                        protocol_version: None,
                    };
                }
            };
            let params = match parse_optional_params::<ToolsListParams>(request.params) {
                Ok(params) => params,
                Err(response) => {
                    return McpHttpOutcome::Response {
                        response: McpResponse::error(Some(request_id), INVALID_PARAMS, response),
                        session_id: session_id.map(str::to_owned),
                        protocol_version: Some(session.protocol_version),
                    };
                }
            };
            let _ = params.meta.as_ref();
            if params.cursor.is_some() {
                return McpHttpOutcome::Response {
                    response: McpResponse::error(
                        Some(request_id),
                        INVALID_PARAMS,
                        "cursor pagination is not supported",
                    ),
                    session_id: session_id.map(str::to_owned),
                    protocol_version: Some(session.protocol_version),
                };
            }

            let tools = filter_tools_by_toolset(tool_definitions(), &params.toolsets);
            McpHttpOutcome::Response {
                response: McpResponse::success(request_id, json!({ "tools": tools })),
                session_id: session_id.map(str::to_owned),
                protocol_version: Some(session.protocol_version),
            }
        }
        "prompts/list" => {
            let session = match require_session(state, session_id, protocol_version_header).await {
                Ok(session) => session,
                Err(response) => {
                    return McpHttpOutcome::Response {
                        response,
                        session_id: None,
                        protocol_version: None,
                    };
                }
            };
            let params = match parse_optional_params::<PromptsListParams>(request.params) {
                Ok(params) => params,
                Err(response) => {
                    return McpHttpOutcome::Response {
                        response: McpResponse::error(Some(request_id), INVALID_PARAMS, response),
                        session_id: session_id.map(str::to_owned),
                        protocol_version: Some(session.protocol_version),
                    };
                }
            };
            let _ = params.meta.as_ref();
            if params.cursor.is_some() {
                return McpHttpOutcome::Response {
                    response: McpResponse::error(
                        Some(request_id),
                        INVALID_PARAMS,
                        "cursor pagination is not supported",
                    ),
                    session_id: session_id.map(str::to_owned),
                    protocol_version: Some(session.protocol_version),
                };
            }

            McpHttpOutcome::Response {
                response: McpResponse::success(
                    request_id,
                    json!({ "prompts": prompt_definitions() }),
                ),
                session_id: session_id.map(str::to_owned),
                protocol_version: Some(session.protocol_version),
            }
        }
        "resources/list" => {
            let session = match require_session(state, session_id, protocol_version_header).await {
                Ok(session) => session,
                Err(response) => {
                    return McpHttpOutcome::Response {
                        response,
                        session_id: None,
                        protocol_version: None,
                    };
                }
            };
            let params = match parse_optional_params::<ResourcesListParams>(request.params) {
                Ok(params) => params,
                Err(response) => {
                    return McpHttpOutcome::Response {
                        response: McpResponse::error(Some(request_id), INVALID_PARAMS, response),
                        session_id: session_id.map(str::to_owned),
                        protocol_version: Some(session.protocol_version),
                    };
                }
            };
            let _ = params.meta.as_ref();
            if params.cursor.is_some() {
                return McpHttpOutcome::Response {
                    response: McpResponse::error(
                        Some(request_id),
                        INVALID_PARAMS,
                        "cursor pagination is not supported",
                    ),
                    session_id: session_id.map(str::to_owned),
                    protocol_version: Some(session.protocol_version),
                };
            }

            match list_resources(state).await {
                Ok(resources) => McpHttpOutcome::Response {
                    response: McpResponse::success(request_id, json!({ "resources": resources })),
                    session_id: session_id.map(str::to_owned),
                    protocol_version: Some(session.protocol_version),
                },
                Err(message) => McpHttpOutcome::Response {
                    response: McpResponse::error(Some(request_id), INTERNAL_ERROR, message),
                    session_id: session_id.map(str::to_owned),
                    protocol_version: Some(session.protocol_version),
                },
            }
        }
        "resources/templates/list" => {
            let session = match require_session(state, session_id, protocol_version_header).await {
                Ok(session) => session,
                Err(response) => {
                    return McpHttpOutcome::Response {
                        response,
                        session_id: None,
                        protocol_version: None,
                    };
                }
            };
            let params = match parse_optional_params::<ResourceTemplatesListParams>(request.params)
            {
                Ok(params) => params,
                Err(response) => {
                    return McpHttpOutcome::Response {
                        response: McpResponse::error(Some(request_id), INVALID_PARAMS, response),
                        session_id: session_id.map(str::to_owned),
                        protocol_version: Some(session.protocol_version),
                    };
                }
            };
            let _ = params.meta.as_ref();
            if params.cursor.is_some() {
                return McpHttpOutcome::Response {
                    response: McpResponse::error(
                        Some(request_id),
                        INVALID_PARAMS,
                        "cursor pagination is not supported",
                    ),
                    session_id: session_id.map(str::to_owned),
                    protocol_version: Some(session.protocol_version),
                };
            }

            McpHttpOutcome::Response {
                response: McpResponse::success(
                    request_id,
                    json!({ "resourceTemplates": resource_template_definitions() }),
                ),
                session_id: session_id.map(str::to_owned),
                protocol_version: Some(session.protocol_version),
            }
        }
        "resources/read" => {
            let session = match require_session(state, session_id, protocol_version_header).await {
                Ok(session) => session,
                Err(response) => {
                    return McpHttpOutcome::Response {
                        response,
                        session_id: None,
                        protocol_version: None,
                    };
                }
            };
            let params = match parse_params::<ResourceReadParams>(request.params) {
                Ok(params) => params,
                Err(message) => {
                    return McpHttpOutcome::Response {
                        response: McpResponse::error(Some(request_id), INVALID_PARAMS, message),
                        session_id: session_id.map(str::to_owned),
                        protocol_version: Some(session.protocol_version),
                    };
                }
            };
            let _ = params.meta.as_ref();

            match read_resource(state, &params.uri).await {
                Ok(contents) => McpHttpOutcome::Response {
                    response: McpResponse::success(request_id, json!({ "contents": [contents] })),
                    session_id: session_id.map(str::to_owned),
                    protocol_version: Some(session.protocol_version),
                },
                Err(ResourceReadError::NotFound(message)) => McpHttpOutcome::Response {
                    response: McpResponse::error(Some(request_id), INVALID_PARAMS, message),
                    session_id: session_id.map(str::to_owned),
                    protocol_version: Some(session.protocol_version),
                },
                Err(ResourceReadError::Internal(message)) => McpHttpOutcome::Response {
                    response: McpResponse::error(Some(request_id), INTERNAL_ERROR, message),
                    session_id: session_id.map(str::to_owned),
                    protocol_version: Some(session.protocol_version),
                },
            }
        }
        "prompts/get" => {
            let session = match require_session(state, session_id, protocol_version_header).await {
                Ok(session) => session,
                Err(response) => {
                    return McpHttpOutcome::Response {
                        response,
                        session_id: None,
                        protocol_version: None,
                    };
                }
            };
            let params = match parse_params::<PromptGetParams>(request.params) {
                Ok(params) => params,
                Err(message) => {
                    return McpHttpOutcome::Response {
                        response: McpResponse::error(Some(request_id), INVALID_PARAMS, message),
                        session_id: session_id.map(str::to_owned),
                        protocol_version: Some(session.protocol_version),
                    };
                }
            };
            let _ = &params.arguments;
            let _ = params.meta.as_ref();

            match prompt_result(&params.name) {
                Some(result) => McpHttpOutcome::Response {
                    response: McpResponse::success(
                        request_id,
                        serde_json::to_value(result).unwrap(),
                    ),
                    session_id: session_id.map(str::to_owned),
                    protocol_version: Some(session.protocol_version),
                },
                None => McpHttpOutcome::Response {
                    response: McpResponse::error(
                        Some(request_id),
                        INVALID_PARAMS,
                        format!("prompt '{}' is not available", params.name),
                    ),
                    session_id: session_id.map(str::to_owned),
                    protocol_version: Some(session.protocol_version),
                },
            }
        }
        "completion/complete" => {
            let session = match require_session(state, session_id, protocol_version_header).await {
                Ok(session) => session,
                Err(response) => {
                    return McpHttpOutcome::Response {
                        response,
                        session_id: None,
                        protocol_version: None,
                    };
                }
            };
            let params = match parse_params::<CompletionCompleteParams>(request.params) {
                Ok(params) => params,
                Err(message) => {
                    return McpHttpOutcome::Response {
                        response: McpResponse::error(Some(request_id), INVALID_PARAMS, message),
                        session_id: session_id.map(str::to_owned),
                        protocol_version: Some(session.protocol_version),
                    };
                }
            };

            match complete_mcp_argument(state, params).await {
                Ok(result) => McpHttpOutcome::Response {
                    response: McpResponse::success(
                        request_id,
                        serde_json::to_value(result).unwrap(),
                    ),
                    session_id: session_id.map(str::to_owned),
                    protocol_version: Some(session.protocol_version),
                },
                Err(message) => McpHttpOutcome::Response {
                    response: McpResponse::error(Some(request_id), INVALID_PARAMS, message),
                    session_id: session_id.map(str::to_owned),
                    protocol_version: Some(session.protocol_version),
                },
            }
        }
        "logging/setLevel" => {
            let session = match require_session(state, session_id, protocol_version_header).await {
                Ok(session) => session,
                Err(response) => {
                    return McpHttpOutcome::Response {
                        response,
                        session_id: None,
                        protocol_version: None,
                    };
                }
            };
            let params = match parse_params::<LoggingSetLevelParams>(request.params) {
                Ok(params) => params,
                Err(message) => {
                    return McpHttpOutcome::Response {
                        response: McpResponse::error(Some(request_id), INVALID_PARAMS, message),
                        session_id: session_id.map(str::to_owned),
                        protocol_version: Some(session.protocol_version),
                    };
                }
            };
            let _ = params.meta.as_ref();
            if !is_supported_mcp_log_level(&params.level) {
                return McpHttpOutcome::Response {
                    response: McpResponse::error(
                        Some(request_id),
                        INVALID_PARAMS,
                        "unsupported log level",
                    ),
                    session_id: session_id.map(str::to_owned),
                    protocol_version: Some(session.protocol_version),
                };
            }

            McpHttpOutcome::Response {
                response: McpResponse::success(request_id, json!({})),
                session_id: session_id.map(str::to_owned),
                protocol_version: Some(session.protocol_version),
            }
        }
        "tools/call" => {
            let session = match require_session(state, session_id, protocol_version_header).await {
                Ok(session) => session,
                Err(response) => {
                    return McpHttpOutcome::Response {
                        response,
                        session_id: None,
                        protocol_version: None,
                    };
                }
            };
            let mut params = match parse_params::<ToolCallParams>(request.params) {
                Ok(params) => params,
                Err(message) => {
                    return McpHttpOutcome::Response {
                        response: McpResponse::error(Some(request_id), INVALID_PARAMS, message),
                        session_id: session_id.map(str::to_owned),
                        protocol_version: Some(session.protocol_version),
                    };
                }
            };
            let _ = params.meta.as_ref();
            if let Some(reason) = high_risk_tool_reason(&params.name, &params.arguments) {
                let expected = expected_confirmation_token(&params.name, &reason);
                if confirmation_token_from_arguments(&params.arguments) != Some(expected.as_str()) {
                    return McpHttpOutcome::Response {
                        response: McpResponse::error(
                            Some(request_id),
                            INVALID_PARAMS,
                            format!(
                                "confirmation required: {reason}. Retry with confirmationToken '{expected}'"
                            ),
                        ),
                        session_id: session_id.map(str::to_owned),
                        protocol_version: Some(session.protocol_version),
                    };
                }
                remove_confirmation_token(&mut params.arguments);
            }

            let tool_name = params.name.clone();
            info!(
                tool_name = %tool_name,
                session_id = session_id.unwrap_or_default(),
                "mcp tool call started"
            );

            match execute_tool(state, params).await {
                Ok(result) => McpHttpOutcome::Response {
                    response: {
                        info!(tool_name = %tool_name, "mcp tool call completed");
                        McpResponse::success(request_id, serde_json::to_value(result).unwrap())
                    },
                    session_id: session_id.map(str::to_owned),
                    protocol_version: Some(session.protocol_version),
                },
                Err(response) => McpHttpOutcome::Response {
                    response: McpResponse::error(Some(request_id), INTERNAL_ERROR, response),
                    session_id: session_id.map(str::to_owned),
                    protocol_version: Some(session.protocol_version),
                },
            }
        }
        _ => McpHttpOutcome::Response {
            response: McpResponse::error(Some(request_id), METHOD_NOT_FOUND, "method not found"),
            session_id: session_id.map(str::to_owned),
            protocol_version: None,
        },
    }
}

pub async fn delete_session(state: &AppState, session_id: Option<&str>) -> bool {
    let Some(session_id) = session_id else {
        return false;
    };
    state
        .mcp_sessions
        .write()
        .await
        .remove(session_id)
        .is_some()
}

async fn handle_initialize(
    state: &AppState,
    request_id: Value,
    params: Option<Value>,
) -> McpHttpOutcome {
    let params = match parse_params::<InitializeParams>(params) {
        Ok(params) => params,
        Err(message) => {
            return McpHttpOutcome::Response {
                response: McpResponse::error(Some(request_id), INVALID_PARAMS, message),
                session_id: None,
                protocol_version: None,
            };
        }
    };

    if !SUPPORTED_PROTOCOL_VERSIONS.contains(&params.protocol_version.as_str()) {
        return McpHttpOutcome::Response {
            response: McpResponse::error(
                Some(request_id),
                INVALID_PARAMS,
                format!(
                    "unsupported protocolVersion '{}'; supported versions: {}",
                    params.protocol_version,
                    SUPPORTED_PROTOCOL_VERSIONS.join(", ")
                ),
            ),
            session_id: None,
            protocol_version: None,
        };
    }

    if let Some(client_info) = params.client_info.as_ref() {
        info!(
            client_name = client_info.name,
            client_version = client_info.version,
            protocol_version = params.protocol_version,
            "mcp client initialized"
        );
    }
    let _ = params.meta.as_ref();
    if !params.capabilities.is_null() {
        info!(capabilities = %params.capabilities, "mcp client capabilities received");
    }

    let session_id = new_uuid_v7();
    state.mcp_sessions.write().await.insert(
        session_id.clone(),
        McpSession {
            protocol_version: params.protocol_version.clone(),
        },
    );

    McpHttpOutcome::Response {
        response: McpResponse::success(
            request_id,
            json!({
                "protocolVersion": params.protocol_version,
                "capabilities": {
                    "prompts": {
                        "listChanged": false
                    },
                    "resources": {
                        "listChanged": false,
                        "subscribe": false
                    },
                    "completions": {},
                    "logging": {},
                    "tools": {
                        "listChanged": false
                    }
                },
                "serverInfo": McpPeerInfo {
                    name: env!("CARGO_PKG_NAME").to_owned(),
                    title: Some("Previa Main MCP".to_owned()),
                    version: env!("CARGO_PKG_VERSION").to_owned(),
                },
                "instructions": "Use the available tools to inspect orchestrator health, projects, pipelines, execution history, queues, OpenAPI specs, and live HTTP behavior. Use the available prompts when you need guidance for project onboarding, pipeline authoring, failure triage, step repair planning, OpenAPI ingestion, load-test design, queue operations, safe reviews, migrations, and spec-driven pipeline bootstrapping."
            }),
        ),
        session_id: Some(session_id),
        protocol_version: Some(params.protocol_version),
    }
}

#[derive(Debug)]
enum ResourceReadError {
    NotFound(String),
    Internal(String),
}

fn resource_template_definitions() -> Vec<ResourceTemplateDefinition> {
    vec![
        ResourceTemplateDefinition {
            uri_template: "previa://projects/{projectId}".to_owned(),
            name: "project".to_owned(),
            title: Some("Project".to_owned()),
            description: Some("Project metadata by id.".to_owned()),
            mime_type: Some("application/json".to_owned()),
        },
        ResourceTemplateDefinition {
            uri_template: "previa://projects/{projectId}/pipelines/{pipelineRef}".to_owned(),
            name: "project-pipeline".to_owned(),
            title: Some("Project Pipeline".to_owned()),
            description: Some("Pipeline by id:{pipelineId} or index:{pipelineIndex}.".to_owned()),
            mime_type: Some("application/json".to_owned()),
        },
        ResourceTemplateDefinition {
            uri_template: "previa://projects/{projectId}/specs/{specId}".to_owned(),
            name: "project-spec".to_owned(),
            title: Some("Project Spec".to_owned()),
            description: Some("Saved OpenAPI spec record by id.".to_owned()),
            mime_type: Some("application/json".to_owned()),
        },
        ResourceTemplateDefinition {
            uri_template: "previa://projects/{projectId}/history/e2e/{testId}".to_owned(),
            name: "project-e2e-test".to_owned(),
            title: Some("E2E Test History Record".to_owned()),
            description: Some("Stored E2E execution result by test id.".to_owned()),
            mime_type: Some("application/json".to_owned()),
        },
        ResourceTemplateDefinition {
            uri_template: "previa://projects/{projectId}/history/load/{testId}".to_owned(),
            name: "project-load-test".to_owned(),
            title: Some("Load Test History Record".to_owned()),
            description: Some("Stored load execution result by test id.".to_owned()),
            mime_type: Some("application/json".to_owned()),
        },
    ]
}

async fn list_resources(state: &AppState) -> Result<Vec<ResourceDefinition>, String> {
    let projects = list_project_records(
        &state.db,
        ProjectListQuery {
            limit: Some(500),
            offset: Some(0),
            order: Some(HistoryOrder::Desc),
        },
    )
    .await
    .map_err(|err| format!("failed to list projects for resources: {err}"))?;

    let mut resources = vec![
        ResourceDefinition {
            uri: "previa://openapi".to_owned(),
            name: "openapi".to_owned(),
            title: Some("OpenAPI Document".to_owned()),
            description: Some("Current orchestrator OpenAPI document.".to_owned()),
            mime_type: Some("application/json".to_owned()),
        },
        ResourceDefinition {
            uri: orchestrator_info_resource_uri().to_owned(),
            name: "orchestrator-info".to_owned(),
            title: Some("Orchestrator Info".to_owned()),
            description: Some("Current orchestrator and runner health.".to_owned()),
            mime_type: Some("application/json".to_owned()),
        },
        ResourceDefinition {
            uri: runners_resource_uri().to_owned(),
            name: "runners".to_owned(),
            title: Some("Runners".to_owned()),
            description: Some("Registered and active runner state.".to_owned()),
            mime_type: Some("application/json".to_owned()),
        },
    ];

    for project in projects {
        resources.push(ResourceDefinition {
            uri: project_resource_uri(&project.id),
            name: format!("project-{}", project.id),
            title: Some(project.name.clone()),
            description: Some("Project metadata record.".to_owned()),
            mime_type: Some("application/json".to_owned()),
        });
        resources.push(ResourceDefinition {
            uri: project_pipelines_resource_uri(&project.id),
            name: format!("project-{}-pipelines", project.id),
            title: Some(format!("{} Pipelines", project.name)),
            description: Some("Saved pipelines for the project.".to_owned()),
            mime_type: Some("application/json".to_owned()),
        });
        resources.push(ResourceDefinition {
            uri: project_specs_resource_uri(&project.id),
            name: format!("project-{}-specs", project.id),
            title: Some(format!("{} Specs", project.name)),
            description: Some("Saved OpenAPI specs for the project.".to_owned()),
            mime_type: Some("application/json".to_owned()),
        });
        resources.push(ResourceDefinition {
            uri: project_e2e_history_resource_uri(&project.id),
            name: format!("project-{}-e2e-history", project.id),
            title: Some(format!("{} E2E History", project.name)),
            description: Some("Recent E2E executions for the project.".to_owned()),
            mime_type: Some("application/json".to_owned()),
        });
        resources.push(ResourceDefinition {
            uri: project_load_history_resource_uri(&project.id),
            name: format!("project-{}-load-history", project.id),
            title: Some(format!("{} Load History", project.name)),
            description: Some("Recent load-test executions for the project.".to_owned()),
            mime_type: Some("application/json".to_owned()),
        });
        resources.push(ResourceDefinition {
            uri: project_current_e2e_queue_resource_uri(&project.id),
            name: format!("project-{}-current-e2e-queue", project.id),
            title: Some(format!("{} Current E2E Queue", project.name)),
            description: Some("Current E2E queue snapshot when one is active.".to_owned()),
            mime_type: Some("application/json".to_owned()),
        });

        let pipelines = load_pipelines_for_project(&state.db, &project.id)
            .await
            .map_err(|err| format!("failed to list project pipelines for resources: {err}"))?;
        for (index, pipeline) in pipelines.iter().enumerate() {
            let segment = pipeline_resource_segment(pipeline, index);
            resources.push(ResourceDefinition {
                uri: format!(
                    "{}/{}",
                    project_pipelines_resource_uri(&project.id),
                    segment
                ),
                name: format!("project-{}-pipeline-{}", project.id, segment),
                title: Some(pipeline.name.clone()),
                description: Some("Saved pipeline definition.".to_owned()),
                mime_type: Some("application/json".to_owned()),
            });
        }

        let specs = list_project_spec_records(&state.db, &project.id)
            .await
            .map_err(|err| format!("failed to list project specs for resources: {err}"))?;
        for spec in specs {
            resources.push(ResourceDefinition {
                uri: format!("{}/{}", project_specs_resource_uri(&project.id), spec.id),
                name: format!("project-{}-spec-{}", project.id, spec.id),
                title: spec
                    .slug
                    .clone()
                    .or_else(|| spec.url.clone())
                    .or_else(|| Some(spec.id.clone())),
                description: Some("Saved project spec record.".to_owned()),
                mime_type: Some("application/json".to_owned()),
            });
        }
    }

    Ok(resources)
}

async fn read_resource(state: &AppState, uri: &str) -> Result<ResourceContents, ResourceReadError> {
    if uri == "previa://openapi" {
        let document = build_openapi_document();
        return json_resource(uri, &document).map_err(|err| {
            ResourceReadError::Internal(format!("failed to encode resource: {err}"))
        });
    }
    if uri == orchestrator_info_resource_uri() {
        let info = build_orchestrator_info(state)
            .await
            .map_err(ResourceReadError::Internal)?;
        return json_resource(uri, &info).map_err(|err| {
            ResourceReadError::Internal(format!("failed to encode resource: {err}"))
        });
    }
    if uri == runners_resource_uri() {
        let info = build_orchestrator_info(state)
            .await
            .map_err(ResourceReadError::Internal)?;
        return json_resource(uri, &info.runners).map_err(|err| {
            ResourceReadError::Internal(format!("failed to encode resource: {err}"))
        });
    }

    let path = uri
        .strip_prefix("previa://")
        .ok_or_else(|| ResourceReadError::NotFound(format!("resource '{uri}' is not available")))?;
    let segments = path.split('/').collect::<Vec<_>>();

    match segments.as_slice() {
        ["projects", project_id] => {
            let project = load_project_record(&state.db, project_id)
                .await
                .map_err(|err| {
                    ResourceReadError::Internal(format!("failed to load project resource: {err}"))
                })?
                .ok_or_else(|| {
                    ResourceReadError::NotFound(format!("project resource '{uri}' was not found"))
                })?;
            json_resource(uri, &project).map_err(|err| {
                ResourceReadError::Internal(format!("failed to encode resource: {err}"))
            })
        }
        ["projects", project_id, "pipelines"] => {
            if !project_exists(&state.db, project_id).await.map_err(|err| {
                ResourceReadError::Internal(format!("failed to verify project resource: {err}"))
            })? {
                return Err(ResourceReadError::NotFound(format!(
                    "project resource '{uri}' was not found"
                )));
            }
            let pipelines = load_pipelines_for_project(&state.db, project_id)
                .await
                .map_err(|err| {
                    ResourceReadError::Internal(format!("failed to load pipeline resources: {err}"))
                })?;
            json_resource(uri, &pipelines).map_err(|err| {
                ResourceReadError::Internal(format!("failed to encode resource: {err}"))
            })
        }
        ["projects", project_id, "pipelines", pipeline_ref] => {
            if !project_exists(&state.db, project_id).await.map_err(|err| {
                ResourceReadError::Internal(format!("failed to verify project resource: {err}"))
            })? {
                return Err(ResourceReadError::NotFound(format!(
                    "project pipeline resource '{uri}' was not found"
                )));
            }
            let pipeline = load_pipeline_resource(state, project_id, pipeline_ref).await?;
            json_resource(uri, &pipeline).map_err(|err| {
                ResourceReadError::Internal(format!("failed to encode resource: {err}"))
            })
        }
        ["projects", project_id, "specs"] => {
            if !project_exists(&state.db, project_id).await.map_err(|err| {
                ResourceReadError::Internal(format!("failed to verify project resource: {err}"))
            })? {
                return Err(ResourceReadError::NotFound(format!(
                    "project resource '{uri}' was not found"
                )));
            }
            let specs = list_project_spec_records(&state.db, project_id)
                .await
                .map_err(|err| {
                    ResourceReadError::Internal(format!("failed to load spec resources: {err}"))
                })?;
            json_resource(uri, &specs).map_err(|err| {
                ResourceReadError::Internal(format!("failed to encode resource: {err}"))
            })
        }
        ["projects", project_id, "specs", spec_id] => {
            let spec = load_project_spec_record_by_id(&state.db, project_id, spec_id)
                .await
                .map_err(|err| {
                    ResourceReadError::Internal(format!("failed to load spec resource: {err}"))
                })?
                .ok_or_else(|| {
                    ResourceReadError::NotFound(format!(
                        "project spec resource '{uri}' was not found"
                    ))
                })?;
            json_resource(uri, &spec).map_err(|err| {
                ResourceReadError::Internal(format!("failed to encode resource: {err}"))
            })
        }
        ["projects", project_id, "history", "e2e"] => {
            ensure_project_resource_exists(&state.db, project_id, uri).await?;
            let records = list_e2e_history_records(
                &state.db,
                project_id,
                HistoryQuery {
                    pipeline_index: None,
                    limit: Some(100),
                    offset: Some(0),
                    order: Some(HistoryOrder::Desc),
                },
            )
            .await
            .map_err(|err| {
                ResourceReadError::Internal(format!("failed to load e2e history resource: {err}"))
            })?;
            json_resource(uri, &records).map_err(|err| {
                ResourceReadError::Internal(format!("failed to encode resource: {err}"))
            })
        }
        ["projects", project_id, "history", "e2e", test_id] => {
            let record = load_e2e_history_record_by_id(&state.db, project_id, test_id)
                .await
                .map_err(|err| {
                    ResourceReadError::Internal(format!("failed to load e2e test resource: {err}"))
                })?
                .ok_or_else(|| {
                    ResourceReadError::NotFound(format!("e2e test resource '{uri}' was not found"))
                })?;
            json_resource(uri, &record).map_err(|err| {
                ResourceReadError::Internal(format!("failed to encode resource: {err}"))
            })
        }
        ["projects", project_id, "history", "load"] => {
            ensure_project_resource_exists(&state.db, project_id, uri).await?;
            let records = list_load_history_records(
                &state.db,
                project_id,
                HistoryQuery {
                    pipeline_index: None,
                    limit: Some(100),
                    offset: Some(0),
                    order: Some(HistoryOrder::Desc),
                },
            )
            .await
            .map_err(|err| {
                ResourceReadError::Internal(format!("failed to load load history resource: {err}"))
            })?;
            json_resource(uri, &records).map_err(|err| {
                ResourceReadError::Internal(format!("failed to encode resource: {err}"))
            })
        }
        ["projects", project_id, "history", "load", test_id] => {
            let record = load_load_history_record_by_id(&state.db, project_id, test_id)
                .await
                .map_err(|err| {
                    ResourceReadError::Internal(format!("failed to load load test resource: {err}"))
                })?
                .ok_or_else(|| {
                    ResourceReadError::NotFound(format!("load test resource '{uri}' was not found"))
                })?;
            json_resource(uri, &record).map_err(|err| {
                ResourceReadError::Internal(format!("failed to encode resource: {err}"))
            })
        }
        ["projects", project_id, "queues", "e2e", "current"] => {
            ensure_project_resource_exists(&state.db, project_id, uri).await?;
            let snapshot = get_current_e2e_queue_snapshot(state, project_id)
                .await
                .map_err(|err| match err {
                    QueueError::NotFound(message) => ResourceReadError::NotFound(format!(
                        "project queue resource '{uri}' was not found: {message}"
                    )),
                    err => ResourceReadError::Internal(format!(
                        "failed to load queue resource: {err:?}"
                    )),
                })?;
            json_resource(uri, &snapshot).map_err(|err| {
                ResourceReadError::Internal(format!("failed to encode resource: {err}"))
            })
        }
        _ => Err(ResourceReadError::NotFound(format!(
            "resource '{uri}' is not available"
        ))),
    }
}

async fn build_orchestrator_info(state: &AppState) -> Result<OrchestratorInfoResponse, String> {
    let runners = crate::server::services::runner_registry::collect_registered_runner_statuses(
        &state.db,
        &state.client,
        state.runner_auth_key.as_deref(),
    )
    .await
    .map_err(|err| format!("failed to load runner registry: {err}"))?;

    Ok(OrchestratorInfoResponse {
        context: state.context_name.clone(),
        total_runners: runners.len(),
        active_runners: runners.iter().filter(|runner| runner.active).count(),
        runners,
    })
}

async fn ensure_project_resource_exists(
    db: &DbPool,
    project_id: &str,
    uri: &str,
) -> Result<(), ResourceReadError> {
    if project_exists(db, project_id).await.map_err(|err| {
        ResourceReadError::Internal(format!("failed to verify project resource: {err}"))
    })? {
        return Ok(());
    }

    Err(ResourceReadError::NotFound(format!(
        "project resource '{uri}' was not found"
    )))
}

async fn load_pipeline_resource(
    state: &AppState,
    project_id: &str,
    pipeline_ref: &str,
) -> Result<Pipeline, ResourceReadError> {
    if let Some(id) = pipeline_ref.strip_prefix("id:") {
        return load_project_pipeline_record(&state.db, project_id, id)
            .await
            .map_err(|err| {
                ResourceReadError::Internal(format!("failed to load pipeline resource: {err}"))
            })?
            .ok_or_else(|| {
                ResourceReadError::NotFound(format!(
                    "project pipeline resource '{}' was not found",
                    format!(
                        "{}/{}",
                        project_pipelines_resource_uri(project_id),
                        pipeline_ref
                    )
                ))
            });
    }

    if let Some(index) = pipeline_ref.strip_prefix("index:") {
        let index = index.parse::<usize>().map_err(|_| {
            ResourceReadError::NotFound(format!(
                "project pipeline resource '{}' was not found",
                format!(
                    "{}/{}",
                    project_pipelines_resource_uri(project_id),
                    pipeline_ref
                )
            ))
        })?;
        let pipelines = load_pipelines_for_project(&state.db, project_id)
            .await
            .map_err(|err| {
                ResourceReadError::Internal(format!("failed to load pipeline resources: {err}"))
            })?;
        return pipelines.get(index).cloned().ok_or_else(|| {
            ResourceReadError::NotFound(format!(
                "project pipeline resource '{}' was not found",
                format!(
                    "{}/{}",
                    project_pipelines_resource_uri(project_id),
                    pipeline_ref
                )
            ))
        });
    }

    Err(ResourceReadError::NotFound(format!(
        "project pipeline resource '{}' was not found",
        format!(
            "{}/{}",
            project_pipelines_resource_uri(project_id),
            pipeline_ref
        )
    )))
}

fn json_resource(
    uri: &str,
    value: &impl serde::Serialize,
) -> Result<ResourceContents, serde_json::Error> {
    Ok(ResourceContents {
        uri: uri.to_owned(),
        mime_type: Some("application/json".to_owned()),
        text: serde_json::to_string_pretty(value)?,
    })
}

fn orchestrator_info_resource_uri() -> &'static str {
    "previa://orchestrator/info"
}

fn runners_resource_uri() -> &'static str {
    "previa://runners"
}

fn project_resource_uri(project_id: &str) -> String {
    format!("previa://projects/{project_id}")
}

fn project_pipelines_resource_uri(project_id: &str) -> String {
    format!("{}/pipelines", project_resource_uri(project_id))
}

fn project_specs_resource_uri(project_id: &str) -> String {
    format!("{}/specs", project_resource_uri(project_id))
}

fn project_e2e_history_resource_uri(project_id: &str) -> String {
    format!("{}/history/e2e", project_resource_uri(project_id))
}

fn project_load_history_resource_uri(project_id: &str) -> String {
    format!("{}/history/load", project_resource_uri(project_id))
}

fn project_current_e2e_queue_resource_uri(project_id: &str) -> String {
    format!("{}/queues/e2e/current", project_resource_uri(project_id))
}

fn pipeline_resource_segment(pipeline: &Pipeline, index: usize) -> String {
    pipeline
        .id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| format!("id:{id}"))
        .unwrap_or_else(|| format!("index:{index}"))
}

async fn complete_mcp_argument(
    state: &AppState,
    params: CompletionCompleteParams,
) -> Result<CompletionResult, String> {
    let _ = params.meta.as_ref();
    let values = match params.reference {
        CompletionReference::Resource { uri } => {
            complete_resource_argument(
                state,
                &uri,
                &params.argument.name,
                &params.argument.value,
                &params.context,
            )
            .await?
        }
        CompletionReference::Prompt { name } => {
            complete_prompt_argument(&name, &params.argument.name, &params.argument.value)
        }
    };
    let total = values.len();
    Ok(CompletionResult {
        completion: CompletionValues {
            values,
            total: Some(total),
            has_more: false,
        },
    })
}

async fn complete_resource_argument(
    state: &AppState,
    uri_template: &str,
    argument_name: &str,
    partial: &str,
    context: &CompletionContext,
) -> Result<Vec<String>, String> {
    match argument_name {
        "projectId" => {
            let projects = list_project_records(
                &state.db,
                ProjectListQuery {
                    limit: Some(100),
                    offset: Some(0),
                    order: Some(HistoryOrder::Desc),
                },
            )
            .await
            .map_err(|err| format!("failed to complete projects: {err}"))?;
            Ok(filter_completion_values(
                projects.into_iter().map(|project| project.id),
                partial,
            ))
        }
        "pipelineRef" => {
            let project_id = completion_context_string(context, "projectId").ok_or_else(|| {
                "projectId context is required to complete pipelineRef".to_owned()
            })?;
            let pipelines = load_pipelines_for_project(&state.db, &project_id)
                .await
                .map_err(|err| format!("failed to complete pipelines: {err}"))?;
            Ok(filter_completion_values(
                pipelines
                    .into_iter()
                    .enumerate()
                    .map(|(index, pipeline)| pipeline_resource_segment(&pipeline, index)),
                partial,
            ))
        }
        "specId" => {
            let project_id = completion_context_string(context, "projectId")
                .ok_or_else(|| "projectId context is required to complete specId".to_owned())?;
            let specs = list_project_spec_records(&state.db, &project_id)
                .await
                .map_err(|err| format!("failed to complete specs: {err}"))?;
            Ok(filter_completion_values(
                specs.into_iter().map(|spec| spec.id),
                partial,
            ))
        }
        "testId" if uri_template.contains("/history/e2e/") => {
            complete_history_test_ids(state, partial, true, context).await
        }
        "testId" if uri_template.contains("/history/load/") => {
            complete_history_test_ids(state, partial, false, context).await
        }
        _ => Ok(Vec::new()),
    }
}

fn complete_prompt_argument(_name: &str, _argument_name: &str, _partial: &str) -> Vec<String> {
    Vec::new()
}

fn completion_context_string(context: &CompletionContext, name: &str) -> Option<String> {
    context
        .arguments
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn filter_completion_values(
    values: impl IntoIterator<Item = String>,
    partial: &str,
) -> Vec<String> {
    values
        .into_iter()
        .filter(|value| value.starts_with(partial))
        .take(100)
        .collect()
}

async fn complete_history_test_ids(
    state: &AppState,
    partial: &str,
    is_e2e: bool,
    context: &CompletionContext,
) -> Result<Vec<String>, String> {
    let project_id = completion_context_string(context, "projectId")
        .ok_or_else(|| "projectId context is required to complete testId".to_owned())?;
    let query = HistoryQuery {
        pipeline_index: None,
        limit: Some(100),
        offset: Some(0),
        order: Some(HistoryOrder::Desc),
    };
    if is_e2e {
        let records = list_e2e_history_records(&state.db, &project_id, query)
            .await
            .map_err(|err| format!("failed to complete e2e tests: {err}"))?;
        Ok(filter_completion_values(
            records.into_iter().map(|record| record.id),
            partial,
        ))
    } else {
        let records = list_load_history_records(&state.db, &project_id, query)
            .await
            .map_err(|err| format!("failed to complete load tests: {err}"))?;
        Ok(filter_completion_values(
            records.into_iter().map(|record| record.id),
            partial,
        ))
    }
}

async fn require_session(
    state: &AppState,
    session_id: Option<&str>,
    protocol_version_header: Option<&str>,
) -> Result<McpSession, McpResponse> {
    let Some(session_id) = session_id else {
        return Err(McpResponse::error(
            None,
            INVALID_SESSION,
            "missing MCP-Session-Id header",
        ));
    };

    let Some(session) = state.mcp_sessions.read().await.get(session_id).cloned() else {
        return Err(McpResponse::error(
            None,
            INVALID_SESSION,
            "unknown MCP session",
        ));
    };

    if let Some(protocol_version) = protocol_version_header {
        if protocol_version != session.protocol_version {
            return Err(McpResponse::error(
                None,
                INVALID_REQUEST,
                format!(
                    "MCP-Protocol-Version header '{}' does not match negotiated session version '{}'",
                    protocol_version, session.protocol_version
                ),
            ));
        }
    }

    Ok(session)
}

async fn execute_tool(state: &AppState, params: ToolCallParams) -> Result<ToolCallResult, String> {
    match params.name.as_str() {
        "health" => Ok(tool_success(json!({ "status": "ok" }))),
        "get_info" => {
            let payload = build_orchestrator_info(state).await?;
            Ok(tool_success(serde_json::to_value(payload).unwrap()))
        }
        "get_openapi_document" => Ok(tool_success(
            serde_json::to_value(build_openapi_document()).unwrap(),
        )),
        "get_pipeline_creation_guide" => Ok(tool_success(pipeline_creation_guide())),
        "list_projects" => {
            let args = parse_tool_arguments::<ListProjectsToolArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            let projects = list_project_records(
                &state.db,
                ProjectListQuery {
                    limit: args.limit,
                    offset: args.offset,
                    order: args.order,
                },
            )
            .await
            .map_err(|err| format!("failed to list projects: {err}"))?;
            Ok(tool_success(serde_json::to_value(projects).unwrap()))
        }
        "get_project" => {
            let args = parse_tool_arguments::<ProjectByIdArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            let project = load_project_record(&state.db, &args.project_id)
                .await
                .map_err(|err| format!("failed to load project: {err}"))?;
            match project {
                Some(project) => Ok(tool_success(serde_json::to_value(project).unwrap())),
                None => Ok(tool_error(format!(
                    "project '{}' not found",
                    args.project_id
                ))),
            }
        }
        "create_project" => {
            let args = parse_tool_arguments::<CreateProjectArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            if args.project.name.trim().is_empty() {
                return Ok(tool_error("project name is required".to_owned()));
            }
            let project = upsert_project_with_pipelines(&state.db, new_uuid_v7(), args.project)
                .await
                .map_err(|err| format!("failed to create project: {err}"))?;
            Ok(tool_success(serde_json::to_value(project).unwrap()))
        }
        "update_project" => {
            let args = parse_tool_arguments::<UpdateProjectArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            if args.project.name.trim().is_empty() {
                return Ok(tool_error("project name is required".to_owned()));
            }
            let project = upsert_project_metadata(&state.db, args.project_id, args.project)
                .await
                .map_err(|err| format!("failed to update project: {err}"))?;
            Ok(tool_success(serde_json::to_value(project).unwrap()))
        }
        "delete_project" => {
            let args = parse_tool_arguments::<ProjectByIdArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            let deleted = sqlx::query("DELETE FROM projects WHERE id = ?")
                .bind(&args.project_id)
                .execute(&state.db)
                .await
                .map_err(|err| format!("failed to delete project: {err}"))?
                .rows_affected()
                > 0;
            if deleted {
                Ok(tool_success(json!({
                    "projectId": args.project_id,
                    "deleted": true
                })))
            } else {
                Ok(tool_error(format!(
                    "project '{}' not found",
                    args.project_id
                )))
            }
        }
        "export_project" => {
            let args = parse_tool_arguments::<ExportProjectArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            let project_id = args.project_id.trim();
            if project_id.is_empty() {
                return Ok(tool_error("projectId cannot be empty".to_owned()));
            }

            let include_history = args.include_history.unwrap_or(true);
            let mut project = match load_project_export(&state.db, project_id)
                .await
                .map_err(|err| format!("failed to load project export: {err}"))?
            {
                Some(project) => project,
                None => return Ok(tool_error(format!("project '{}' not found", project_id))),
            };

            if include_history {
                project.history.e2e = load_e2e_history_for_export(&state.db, project_id)
                    .await
                    .map_err(|err| format!("failed to load e2e history export: {err}"))?;
                project.history.load = load_load_history_for_export(&state.db, project_id)
                    .await
                    .map_err(|err| format!("failed to load load history export: {err}"))?;
            }

            Ok(tool_success(
                serde_json::to_value(ProjectExportEnvelope {
                    format: "previa.project.export.v1".to_owned(),
                    exported_at: now_iso(),
                    history_included: include_history,
                    project,
                })
                .unwrap(),
            ))
        }
        "import_project" => {
            let args = parse_tool_arguments::<ImportProjectArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            let mut bundle = args.bundle;
            if bundle.format != "previa.project.export.v1" {
                return Ok(tool_error("invalid import format".to_owned()));
            }
            bundle.project.id = bundle.project.id.trim().to_owned();
            bundle.project.name = bundle.project.name.trim().to_owned();
            if bundle.project.id.is_empty() {
                return Ok(tool_error("project.id is required".to_owned()));
            }
            if bundle.project.name.is_empty() {
                return Ok(tool_error("project.name is required".to_owned()));
            }
            if project_exists(&state.db, &bundle.project.id)
                .await
                .map_err(|err| format!("failed to load project: {err}"))?
            {
                return Ok(tool_error("project already exists".to_owned()));
            }

            let imported = import_project_bundle(
                &state.db,
                &bundle.project,
                args.include_history.unwrap_or(true),
            )
            .await
            .map_err(|err| format!("failed to import project: {err}"))?;
            Ok(tool_success(serde_json::to_value(imported).unwrap()))
        }
        "list_project_pipelines" => {
            let args = parse_tool_arguments::<ProjectByIdArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            if !project_exists(&state.db, &args.project_id)
                .await
                .map_err(|err| format!("failed to load project: {err}"))?
            {
                return Ok(tool_error(format!(
                    "project '{}' not found",
                    args.project_id
                )));
            }
            let pipelines = load_pipelines_for_project(&state.db, &args.project_id)
                .await
                .map_err(|err| format!("failed to load project pipelines: {err}"))?;
            Ok(tool_success(serde_json::to_value(pipelines).unwrap()))
        }
        "list_e2e_history" => {
            let args = parse_tool_arguments::<ProjectHistoryToolArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            if !project_exists(&state.db, &args.project_id)
                .await
                .map_err(|err| format!("failed to load project: {err}"))?
            {
                return Ok(tool_error(format!(
                    "project '{}' not found",
                    args.project_id
                )));
            }
            let records = list_e2e_history_records(
                &state.db,
                &args.project_id,
                HistoryQuery {
                    pipeline_index: args.pipeline_index,
                    limit: args.limit,
                    offset: args.offset,
                    order: args.order,
                },
            )
            .await
            .map_err(|err| format!("failed to list e2e history: {err}"))?;
            Ok(tool_success(serde_json::to_value(records).unwrap()))
        }
        "get_e2e_test" => {
            let args = parse_tool_arguments::<ProjectTestByIdArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            let record = load_e2e_history_record_by_id(&state.db, &args.project_id, &args.test_id)
                .await
                .map_err(|err| format!("failed to load e2e test: {err}"))?;
            match record {
                Some(record) => Ok(tool_success(serde_json::to_value(record).unwrap())),
                None => Ok(tool_error(format!(
                    "e2e test '{}' not found in project '{}'",
                    args.test_id, args.project_id
                ))),
            }
        }
        "list_load_history" => {
            let args = parse_tool_arguments::<ProjectHistoryToolArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            if !project_exists(&state.db, &args.project_id)
                .await
                .map_err(|err| format!("failed to load project: {err}"))?
            {
                return Ok(tool_error(format!(
                    "project '{}' not found",
                    args.project_id
                )));
            }
            let records = list_load_history_records(
                &state.db,
                &args.project_id,
                HistoryQuery {
                    pipeline_index: args.pipeline_index,
                    limit: args.limit,
                    offset: args.offset,
                    order: args.order,
                },
            )
            .await
            .map_err(|err| format!("failed to list load history: {err}"))?;
            Ok(tool_success(serde_json::to_value(records).unwrap()))
        }
        "get_load_test" => {
            let args = parse_tool_arguments::<ProjectTestByIdArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            let record = load_load_history_record_by_id(&state.db, &args.project_id, &args.test_id)
                .await
                .map_err(|err| format!("failed to load load test: {err}"))?;
            match record {
                Some(record) => Ok(tool_success(serde_json::to_value(record).unwrap())),
                None => Ok(tool_error(format!(
                    "load test '{}' not found in project '{}'",
                    args.test_id, args.project_id
                ))),
            }
        }
        "get_project_pipeline" => {
            let args = parse_tool_arguments::<ProjectPipelineByIdArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            if !project_exists(&state.db, &args.project_id)
                .await
                .map_err(|err| format!("failed to load project: {err}"))?
            {
                return Ok(tool_error(format!(
                    "project '{}' not found",
                    args.project_id
                )));
            }
            let pipeline =
                load_project_pipeline_record(&state.db, &args.project_id, &args.pipeline_id)
                    .await
                    .map_err(|err| format!("failed to load project pipeline: {err}"))?;
            match pipeline {
                Some(pipeline) => Ok(tool_success(
                    serde_json::to_value(
                        build_project_pipeline_record(state, &args.project_id, pipeline).await,
                    )
                    .unwrap(),
                )),
                None => Ok(tool_error(format!(
                    "pipeline '{}' not found in project '{}'",
                    args.pipeline_id, args.project_id
                ))),
            }
        }
        "create_project_pipeline" => {
            let args = parse_tool_arguments::<CreateProjectPipelineArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            validate_pipeline_input(&args.pipeline)?;
            let runtime_specs =
                resolve_runtime_specs_for_execution(&state.db, Some(&args.project_id), &[])
                    .await
                    .map_err(|err| format!("failed to load project specs for validation: {err}"))?;
            let template_errors =
                validate_pipeline_templates(&args.pipeline, runtime_specs.as_deref(), None, None);
            if !template_errors.is_empty() {
                return Ok(tool_error(template_errors.join("; ")));
            }
            if !project_exists(&state.db, &args.project_id)
                .await
                .map_err(|err| format!("failed to load project: {err}"))?
            {
                return Ok(tool_error(format!(
                    "project '{}' not found",
                    args.project_id
                )));
            }
            let pipeline = insert_project_pipeline(&state.db, &args.project_id, args.pipeline)
                .await
                .map_err(|err| format!("failed to create project pipeline: {err}"))?;
            Ok(tool_success(serde_json::to_value(pipeline).unwrap()))
        }
        "update_project_pipeline" => {
            let args = parse_tool_arguments::<UpdateProjectPipelineArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            validate_pipeline_input(&args.pipeline)?;
            let runtime_specs =
                resolve_runtime_specs_for_execution(&state.db, Some(&args.project_id), &[])
                    .await
                    .map_err(|err| format!("failed to load project specs for validation: {err}"))?;
            let template_errors =
                validate_pipeline_templates(&args.pipeline, runtime_specs.as_deref(), None, None);
            if !template_errors.is_empty() {
                return Ok(tool_error(template_errors.join("; ")));
            }
            if !project_exists(&state.db, &args.project_id)
                .await
                .map_err(|err| format!("failed to load project: {err}"))?
            {
                return Ok(tool_error(format!(
                    "project '{}' not found",
                    args.project_id
                )));
            }
            let pipeline = update_project_pipeline(
                &state.db,
                &args.project_id,
                &args.pipeline_id,
                args.pipeline,
            )
            .await
            .map_err(|err| format!("failed to update project pipeline: {err}"))?;
            match pipeline {
                Some(pipeline) => Ok(tool_success(serde_json::to_value(pipeline).unwrap())),
                None => Ok(tool_error(format!(
                    "pipeline '{}' not found in project '{}'",
                    args.pipeline_id, args.project_id
                ))),
            }
        }
        "delete_project_pipeline" => {
            let args = parse_tool_arguments::<ProjectPipelineByIdArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            if !project_exists(&state.db, &args.project_id)
                .await
                .map_err(|err| format!("failed to load project: {err}"))?
            {
                return Ok(tool_error(format!(
                    "project '{}' not found",
                    args.project_id
                )));
            }
            let deleted = delete_pipeline_record(&state.db, &args.project_id, &args.pipeline_id)
                .await
                .map_err(|err| format!("failed to delete project pipeline: {err}"))?;
            if deleted {
                Ok(tool_success(json!({
                    "projectId": args.project_id,
                    "pipelineId": args.pipeline_id,
                    "deleted": true
                })))
            } else {
                Ok(tool_error(format!(
                    "pipeline '{}' not found in project '{}'",
                    args.pipeline_id, args.project_id
                )))
            }
        }
        "list_project_specs" => {
            let args = parse_tool_arguments::<ProjectByIdArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            if !project_exists(&state.db, &args.project_id)
                .await
                .map_err(|err| format!("failed to load project: {err}"))?
            {
                return Ok(tool_error(format!(
                    "project '{}' not found",
                    args.project_id
                )));
            }
            let specs = list_project_spec_records(&state.db, &args.project_id)
                .await
                .map_err(|err| format!("failed to list project specs: {err}"))?;
            Ok(tool_success(serde_json::to_value(specs).unwrap()))
        }
        "get_project_spec" => {
            let args = parse_tool_arguments::<ProjectSpecByIdArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            ensure_project_exists(state, &args.project_id).await?;
            match load_project_spec_record_by_id(&state.db, &args.project_id, &args.spec_id)
                .await
                .map_err(|err| format!("failed to load project spec: {err}"))?
            {
                Some(spec) => Ok(tool_success(serde_json::to_value(spec).unwrap())),
                None => Ok(tool_error(format!(
                    "project spec '{}' not found in project '{}'",
                    args.spec_id, args.project_id
                ))),
            }
        }
        "create_project_spec" => {
            let args = parse_tool_arguments::<CreateProjectSpecArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            ensure_project_exists(state, &args.project_id).await?;
            let payload = normalize_project_spec_payload(args.spec)?;
            let spec = insert_project_spec_record(&state.db, &args.project_id, payload)
                .await
                .map_err(|err| format!("failed to create project spec: {err}"))?;
            Ok(tool_success(serde_json::to_value(spec).unwrap()))
        }
        "update_project_spec" => {
            let args = parse_tool_arguments::<UpdateProjectSpecArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            ensure_project_exists(state, &args.project_id).await?;
            let payload = normalize_project_spec_payload(args.spec)?;
            match update_project_spec_record(&state.db, &args.project_id, &args.spec_id, payload)
                .await
                .map_err(|err| format!("failed to update project spec: {err}"))?
            {
                Some(spec) => Ok(tool_success(serde_json::to_value(spec).unwrap())),
                None => Ok(tool_error(format!(
                    "project spec '{}' not found in project '{}'",
                    args.spec_id, args.project_id
                ))),
            }
        }
        "delete_project_spec" => {
            let args = parse_tool_arguments::<ProjectSpecByIdArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            ensure_project_exists(state, &args.project_id).await?;
            let deleted = delete_project_spec_record(&state.db, &args.project_id, &args.spec_id)
                .await
                .map_err(|err| format!("failed to delete project spec: {err}"))?;
            if deleted {
                Ok(tool_success(json!({
                    "projectId": args.project_id,
                    "specId": args.spec_id,
                    "deleted": true
                })))
            } else {
                Ok(tool_error(format!(
                    "project spec '{}' not found in project '{}'",
                    args.spec_id, args.project_id
                )))
            }
        }
        "create_project_e2e_queue" => {
            let args = parse_tool_arguments::<CreateProjectE2eQueueArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            ensure_project_exists(state, &args.project_id).await?;

            match create_e2e_queue(
                state.clone(),
                args.project_id,
                ProjectE2eQueueRequest {
                    pipeline_ids: args.pipeline_ids,
                    selected_base_url_key: args.selected_base_url_key,
                    selected_env_group_slug: args.selected_env_group_slug,
                    specs: args.specs,
                    env_groups: args.env_groups,
                },
            )
            .await
            {
                Ok(snapshot) => Ok(tool_success(serde_json::to_value(snapshot).unwrap())),
                Err(err) => queue_tool_outcome(err),
            }
        }
        "get_current_project_e2e_queue" => {
            let args = parse_tool_arguments::<ProjectByIdArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            ensure_project_exists(state, &args.project_id).await?;

            match get_current_e2e_queue_snapshot(state, &args.project_id).await {
                Ok(snapshot) => Ok(tool_success(serde_json::to_value(snapshot).unwrap())),
                Err(err) => queue_tool_outcome(err),
            }
        }
        "get_project_e2e_queue" => {
            let args = parse_tool_arguments::<ProjectQueueByIdArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            ensure_project_exists(state, &args.project_id).await?;

            match get_e2e_queue_snapshot(state, &args.project_id, &args.queue_id).await {
                Ok(Some(snapshot)) => Ok(tool_success(serde_json::to_value(snapshot).unwrap())),
                Ok(None) => Ok(tool_error(format!(
                    "e2e queue '{}' not found in project '{}'",
                    args.queue_id, args.project_id
                ))),
                Err(err) => queue_tool_outcome(err),
            }
        }
        "cancel_project_e2e_queue" => {
            let args = parse_tool_arguments::<ProjectQueueByIdArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            ensure_project_exists(state, &args.project_id).await?;

            match cancel_e2e_queue(
                state.clone(),
                args.project_id.clone(),
                args.queue_id.clone(),
            )
            .await
            {
                Ok(()) => Ok(tool_success(json!({
                    "projectId": args.project_id,
                    "queueId": args.queue_id,
                    "cancelled": true
                }))),
                Err(err) => queue_tool_outcome(err),
            }
        }
        "run_project_e2e_test" => {
            let args = parse_tool_arguments::<RunProjectE2eTestArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            let transaction_id = args.transaction_id.clone();
            let payload = resolve_project_e2e_request(state, args).await?;
            match start_e2e_execution(state.clone(), payload, transaction_id).await {
                Ok(started) => Ok(tool_success(
                    execution_started_payload(state, &started.execution_id, "e2e").await,
                )),
                Err(err) => execution_start_tool_outcome(err),
            }
        }
        "run_project_load_test" => {
            let args = parse_tool_arguments::<RunProjectLoadTestArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            let transaction_id = args.transaction_id.clone();
            let payload = resolve_project_load_request(state, args).await?;
            match start_load_execution(state.clone(), payload, transaction_id).await {
                Ok(started) => Ok(tool_success(
                    execution_started_payload(state, &started.execution_id, "load").await,
                )),
                Err(err) => load_execution_start_tool_outcome(err),
            }
        }
        "get_execution" => {
            let args = parse_tool_arguments::<ExecutionByIdArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            ensure_project_exists(state, &args.project_id).await?;
            match execution_snapshot(state, &args.project_id, &args.execution_id).await? {
                Some(snapshot) => Ok(tool_success(snapshot)),
                None => Ok(tool_error(format!(
                    "execution '{}' not found in project '{}'",
                    args.execution_id, args.project_id
                ))),
            }
        }
        "cancel_execution" => {
            let args = parse_tool_arguments::<ExecutionCancelArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            match cancel_execution_payload(state, &args.execution_id).await? {
                Some(payload) => Ok(tool_success(payload)),
                None => Ok(tool_error(
                    "execution not found or already finished".to_owned(),
                )),
            }
        }
        "delete_e2e_history" => {
            let args = parse_tool_arguments::<ProjectHistoryToolArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            ensure_project_exists(state, &args.project_id).await?;
            let deleted = delete_history_rows(
                &state.db,
                "integration_history",
                &args.project_id,
                args.pipeline_index,
            )
            .await?;
            Ok(tool_success(json!({
                "projectId": args.project_id,
                "pipelineIndex": args.pipeline_index,
                "deleted": true,
                "rowsAffected": deleted
            })))
        }
        "delete_e2e_test" => {
            let args = parse_tool_arguments::<ProjectTestByIdArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            ensure_project_exists(state, &args.project_id).await?;
            let deleted = sqlx::query(
                "DELETE FROM integration_history WHERE project_id = ? AND (id = ? OR execution_id = ?)",
            )
            .bind(&args.project_id)
            .bind(&args.test_id)
            .bind(&args.test_id)
            .execute(&state.db)
            .await
            .map_err(|err| format!("failed to delete e2e history record: {err}"))?
            .rows_affected()
                > 0;
            if deleted {
                Ok(tool_success(json!({
                    "projectId": args.project_id,
                    "testId": args.test_id,
                    "deleted": true
                })))
            } else {
                Ok(tool_error(format!(
                    "e2e test '{}' not found in project '{}'",
                    args.test_id, args.project_id
                )))
            }
        }
        "delete_load_history" => {
            let args = parse_tool_arguments::<ProjectHistoryToolArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            ensure_project_exists(state, &args.project_id).await?;
            let deleted = delete_history_rows(
                &state.db,
                "load_history",
                &args.project_id,
                args.pipeline_index,
            )
            .await?;
            Ok(tool_success(json!({
                "projectId": args.project_id,
                "pipelineIndex": args.pipeline_index,
                "deleted": true,
                "rowsAffected": deleted
            })))
        }
        "delete_load_test" => {
            let args = parse_tool_arguments::<ProjectTestByIdArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            ensure_project_exists(state, &args.project_id).await?;
            let deleted = sqlx::query(
                "DELETE FROM load_history WHERE project_id = ? AND (id = ? OR execution_id = ?)",
            )
            .bind(&args.project_id)
            .bind(&args.test_id)
            .bind(&args.test_id)
            .execute(&state.db)
            .await
            .map_err(|err| format!("failed to delete load history record: {err}"))?
            .rows_affected()
                > 0;
            if deleted {
                Ok(tool_success(json!({
                    "projectId": args.project_id,
                    "testId": args.test_id,
                    "deleted": true
                })))
            } else {
                Ok(tool_error(format!(
                    "load test '{}' not found in project '{}'",
                    args.test_id, args.project_id
                )))
            }
        }
        "proxy_request" => {
            let args = parse_tool_arguments::<ProxyToolArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            let payload = render_proxy_request(args.request)?;
            let result = proxy_tool_request(
                state,
                payload,
                args.max_events.unwrap_or(50),
                args.timeout_ms.unwrap_or(5_000),
            )
            .await?;
            Ok(tool_success(result))
        }
        "validate_openapi" => {
            let args = parse_tool_arguments::<ValidateOpenApiToolArgs>(params.arguments)?;
            let _ = args.meta.as_ref();
            let payload = validate_openapi_source(&args.source);
            Ok(tool_success(serde_json::to_value(payload).unwrap()))
        }
        _ => Ok(tool_error(format!(
            "tool '{}' is not available",
            params.name
        ))),
    }
}

fn parse_params<T>(params: Option<Value>) -> Result<T, String>
where
    T: DeserializeOwned,
{
    match params {
        Some(value) => serde_json::from_value(value).map_err(|err| err.to_string()),
        None => Err("params are required".to_owned()),
    }
}

fn parse_optional_params<T>(params: Option<Value>) -> Result<T, String>
where
    T: DeserializeOwned + Default,
{
    match params {
        Some(value) => serde_json::from_value(value).map_err(|err| err.to_string()),
        None => Ok(T::default()),
    }
}

fn parse_tool_arguments<T>(arguments: Value) -> Result<T, String>
where
    T: DeserializeOwned,
{
    serde_json::from_value(arguments).map_err(|err| err.to_string())
}

fn tool_definitions() -> Vec<ToolDefinition> {
    let mut tools = vec![
        ToolDefinition {
            name: "health".to_owned(),
            title: Some("Health".to_owned()),
            description: "Returns a simple health payload for the orchestrator.".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "get_info".to_owned(),
            title: Some("Runner Info".to_owned()),
            description: "Returns runner registration and health information.".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "get_openapi_document".to_owned(),
            title: Some("OpenAPI Document".to_owned()),
            description: "Returns the orchestrator OpenAPI document.".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "get_pipeline_creation_guide".to_owned(),
            title: Some("Pipeline Guide".to_owned()),
            description:
                "Explains how to create a pipeline, with examples and supported template variables."
                    .to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "list_projects".to_owned(),
            title: Some("List Projects".to_owned()),
            description: "Lists projects stored in the orchestrator database.".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "minimum": 0 },
                    "offset": { "type": "integer", "minimum": 0 },
                    "order": { "type": "string", "enum": ["asc", "desc"] }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "get_project".to_owned(),
            title: Some("Get Project".to_owned()),
            description: "Returns a project by its id.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "create_project".to_owned(),
            title: Some("Create Project".to_owned()),
            description: "Creates a project with optional pipelines.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["project"],
                "properties": {
                    "project": { "type": "object" }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "update_project".to_owned(),
            title: Some("Update Project".to_owned()),
            description: "Updates project metadata by id.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId", "project"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 },
                    "project": { "type": "object" }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "delete_project".to_owned(),
            title: Some("Delete Project".to_owned()),
            description: "Deletes a project by id.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "export_project".to_owned(),
            title: Some("Export Project".to_owned()),
            description: "Exports a project bundle, optionally including history.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 },
                    "includeHistory": { "type": "boolean" }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "import_project".to_owned(),
            title: Some("Import Project".to_owned()),
            description: "Imports a project bundle.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["bundle"],
                "properties": {
                    "bundle": { "type": "object" },
                    "includeHistory": { "type": "boolean" }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "list_project_pipelines".to_owned(),
            title: Some("List Pipelines".to_owned()),
            description: "Lists pipelines for a project.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "list_e2e_history".to_owned(),
            title: Some("List E2E History".to_owned()),
            description: "Lists executed E2E tests for a project.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 },
                    "pipelineIndex": { "type": "integer" },
                    "limit": { "type": "integer", "minimum": 1 },
                    "offset": { "type": "integer", "minimum": 0 },
                    "order": { "type": "string", "enum": ["asc", "desc"] }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "get_e2e_test".to_owned(),
            title: Some("Get E2E Test".to_owned()),
            description: "Returns a single executed E2E test by history id or execution id."
                .to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId", "testId"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 },
                    "testId": { "type": "string", "minLength": 1 }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "delete_e2e_history".to_owned(),
            title: Some("Delete E2E History".to_owned()),
            description: "Deletes E2E history for a project, optionally filtered by pipeline index.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 },
                    "pipelineIndex": { "type": "integer" }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "delete_e2e_test".to_owned(),
            title: Some("Delete E2E Test".to_owned()),
            description: "Deletes a single E2E history record by history id or execution id.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId", "testId"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 },
                    "testId": { "type": "string", "minLength": 1 }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "list_load_history".to_owned(),
            title: Some("List Load History".to_owned()),
            description: "Lists executed load tests for a project.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 },
                    "pipelineIndex": { "type": "integer" },
                    "limit": { "type": "integer", "minimum": 1 },
                    "offset": { "type": "integer", "minimum": 0 },
                    "order": { "type": "string", "enum": ["asc", "desc"] }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "get_load_test".to_owned(),
            title: Some("Get Load Test".to_owned()),
            description: "Returns a single executed load test by history id or execution id."
                .to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId", "testId"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 },
                    "testId": { "type": "string", "minLength": 1 }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "delete_load_history".to_owned(),
            title: Some("Delete Load History".to_owned()),
            description: "Deletes load history for a project, optionally filtered by pipeline index.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 },
                    "pipelineIndex": { "type": "integer" }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "delete_load_test".to_owned(),
            title: Some("Delete Load Test".to_owned()),
            description: "Deletes a single load history record by history id or execution id.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId", "testId"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 },
                    "testId": { "type": "string", "minLength": 1 }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "get_project_pipeline".to_owned(),
            title: Some("Get Pipeline".to_owned()),
            description: "Returns a single pipeline from a project.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId", "pipelineId"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 },
                    "pipelineId": { "type": "string", "minLength": 1 }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "create_project_pipeline".to_owned(),
            title: Some("Create Pipeline".to_owned()),
            description: "Creates a pipeline inside a project.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId", "pipeline"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 },
                    "pipeline": pipeline_schema()
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "update_project_pipeline".to_owned(),
            title: Some("Update Pipeline".to_owned()),
            description: "Updates an existing pipeline in a project.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId", "pipelineId", "pipeline"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 },
                    "pipelineId": { "type": "string", "minLength": 1 },
                    "pipeline": pipeline_schema()
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "delete_project_pipeline".to_owned(),
            title: Some("Delete Pipeline".to_owned()),
            description: "Deletes a pipeline from a project.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId", "pipelineId"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 },
                    "pipelineId": { "type": "string", "minLength": 1 }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "list_project_specs".to_owned(),
            title: Some("List Specs".to_owned()),
            description: "Lists OpenAPI specs associated with a project.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "get_project_spec".to_owned(),
            title: Some("Get Spec".to_owned()),
            description: "Returns one OpenAPI spec from a project.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId", "specId"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 },
                    "specId": { "type": "string", "minLength": 1 }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "create_project_spec".to_owned(),
            title: Some("Create Spec".to_owned()),
            description: "Creates an OpenAPI spec for a project.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId", "spec"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 },
                    "spec": { "type": "object" }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "update_project_spec".to_owned(),
            title: Some("Update Spec".to_owned()),
            description: "Updates an OpenAPI spec for a project.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId", "specId", "spec"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 },
                    "specId": { "type": "string", "minLength": 1 },
                    "spec": { "type": "object" }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "delete_project_spec".to_owned(),
            title: Some("Delete Spec".to_owned()),
            description: "Deletes an OpenAPI spec from a project.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId", "specId"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 },
                    "specId": { "type": "string", "minLength": 1 }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "create_project_e2e_queue".to_owned(),
            title: Some("Create E2E Queue".to_owned()),
            description: "Creates and starts a sequential E2E queue for a project.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId", "pipelineIds"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 },
                    "pipelineIds": {
                        "type": "array",
                        "minItems": 1,
                        "items": { "type": "string", "minLength": 1 }
                    },
                    "selectedBaseUrlKey": { "type": ["string", "null"] },
                    "specs": {
                        "type": "array",
                        "items": { "type": "object" }
                    }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "get_current_project_e2e_queue".to_owned(),
            title: Some("Get Current E2E Queue".to_owned()),
            description: "Returns the currently active E2E queue for a project.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "get_project_e2e_queue".to_owned(),
            title: Some("Get E2E Queue".to_owned()),
            description: "Returns an E2E queue snapshot by queue id.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId", "queueId"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 },
                    "queueId": { "type": "string", "minLength": 1 }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "cancel_project_e2e_queue".to_owned(),
            title: Some("Cancel E2E Queue".to_owned()),
            description: "Cancels an E2E queue and clears remaining queued pipelines.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId", "queueId"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 },
                    "queueId": { "type": "string", "minLength": 1 }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "run_project_e2e_test".to_owned(),
            title: Some("Run E2E Test".to_owned()),
            description: "Starts an E2E execution for a project and returns the execution id.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 },
                    "pipelineId": { "type": ["string", "null"] },
                    "pipeline": pipeline_schema(),
                    "selectedBaseUrlKey": { "type": ["string", "null"] },
                    "pipelineIndex": { "type": ["integer", "null"] },
                    "specs": { "type": "array", "items": { "type": "object" } },
                    "transactionId": { "type": ["string", "null"] }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "run_project_load_test".to_owned(),
            title: Some("Run Load Test".to_owned()),
            description: "Starts a load execution for a project and returns the execution id.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId"],
                "oneOf": [
                    { "required": ["config"] },
                    { "required": ["load"] }
                ],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 },
                    "pipelineId": { "type": ["string", "null"] },
                    "pipeline": pipeline_schema(),
                    "config": {
                        "type": "object",
                        "required": ["totalRequests", "concurrency", "rampUpSeconds"],
                        "properties": {
                            "totalRequests": { "type": "integer", "minimum": 1 },
                            "concurrency": { "type": "integer", "minimum": 1 },
                            "rampUpSeconds": { "type": "number", "minimum": 0 }
                        }
                    },
                    "load": {
                        "type": "object",
                        "required": ["points"],
                        "properties": {
                            "points": {
                                "type": "array",
                                "minItems": 2,
                                "items": {
                                    "type": "object",
                                    "required": ["atMs", "intensity"],
                                    "properties": {
                                        "atMs": { "type": "integer", "minimum": 0 },
                                        "intensity": { "type": "number", "minimum": 0, "maximum": 100 }
                                    }
                                }
                            },
                            "interpolation": { "type": "string", "enum": ["smooth", "linear", "step"] },
                            "runnerMaxRps": {
                                "type": "number",
                                "minimum": 1,
                                "maximum": 1000,
                                "description": "Maximum requests per second allowed per runner. Defaults to 600 when omitted."
                            },
                            "gracePeriodMs": { "type": "integer", "minimum": 0 }
                        }
                    },
                    "selectedBaseUrlKey": { "type": ["string", "null"] },
                    "pipelineIndex": { "type": ["integer", "null"] },
                    "specs": { "type": "array", "items": { "type": "object" } },
                    "transactionId": { "type": ["string", "null"] }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "get_execution".to_owned(),
            title: Some("Get Execution".to_owned()),
            description: "Returns the active snapshot or final stored result of an execution.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["projectId", "executionId"],
                "properties": {
                    "projectId": { "type": "string", "minLength": 1 },
                    "executionId": { "type": "string", "minLength": 1 }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "cancel_execution".to_owned(),
            title: Some("Cancel Execution".to_owned()),
            description: "Requests cancellation for an active execution.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["executionId"],
                "properties": {
                    "executionId": { "type": "string", "minLength": 1 }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "proxy_request".to_owned(),
            title: Some("Proxy Request".to_owned()),
            description: "Executes a proxied HTTP request. SSE responses are collected with an event/time limit.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["request"],
                "properties": {
                    "request": { "type": "object" },
                    "maxEvents": { "type": "integer", "minimum": 1 },
                    "timeoutMs": { "type": "integer", "minimum": 1 }
                }
            }),
            output_schema: None,
            meta: None,
        },
        ToolDefinition {
            name: "validate_openapi".to_owned(),
            title: Some("Validate OpenAPI".to_owned()),
            description: "Validates an OpenAPI YAML or JSON document.".to_owned(),
            input_schema: json!({
                "type": "object",
                "required": ["source"],
                "properties": {
                    "source": { "type": "string", "minLength": 1 }
                }
            }),
            output_schema: None,
            meta: None,
        },
    ];
    enrich_tool_definitions(&mut tools);
    tools
}

#[cfg(test)]
fn tool_definition(
    name: &str,
    title: &str,
    description: &str,
    input_schema: Value,
    output_schema: Option<Value>,
    toolset: &str,
) -> ToolDefinition {
    ToolDefinition {
        name: name.to_owned(),
        title: Some(title.to_owned()),
        description: description.to_owned(),
        input_schema,
        output_schema,
        meta: Some(json!({ "previaToolset": toolset })),
    }
}

fn enrich_tool_definitions(tools: &mut [ToolDefinition]) {
    for tool in tools {
        let toolset = tool_toolset(&tool.name);
        tool.meta = Some(json!({ "previaToolset": toolset }));
        tool.output_schema = output_schema_for_tool(&tool.name);
        if high_risk_tool_name(&tool.name) {
            add_confirmation_token_property(&mut tool.input_schema);
        }
    }
}

fn tool_toolset(tool_name: &str) -> &'static str {
    match tool_name {
        "health"
        | "get_info"
        | "get_openapi_document"
        | "get_pipeline_creation_guide"
        | "get_execution"
        | "cancel_execution" => "core",
        "list_projects" | "get_project" | "create_project" | "update_project"
        | "delete_project" | "export_project" | "import_project" => "projects",
        "list_project_pipelines"
        | "get_project_pipeline"
        | "create_project_pipeline"
        | "update_project_pipeline"
        | "delete_project_pipeline" => "pipelines",
        "list_project_specs"
        | "get_project_spec"
        | "create_project_spec"
        | "update_project_spec"
        | "delete_project_spec"
        | "validate_openapi" => "specs",
        "list_e2e_history"
        | "get_e2e_test"
        | "delete_e2e_history"
        | "delete_e2e_test"
        | "create_project_e2e_queue"
        | "get_current_project_e2e_queue"
        | "get_project_e2e_queue"
        | "cancel_project_e2e_queue"
        | "run_project_e2e_test" => "e2e",
        "list_load_history"
        | "get_load_test"
        | "delete_load_history"
        | "delete_load_test"
        | "run_project_load_test" => "load",
        "proxy_request" => "http",
        _ => "core",
    }
}

fn filter_tools_by_toolset(
    tools: Vec<ToolDefinition>,
    requested: &[String],
) -> Vec<ToolDefinition> {
    if requested.is_empty() {
        return tools;
    }

    tools
        .into_iter()
        .filter(|tool| {
            tool.meta
                .as_ref()
                .and_then(|meta| meta.get("previaToolset"))
                .and_then(Value::as_str)
                .map(|toolset| requested.iter().any(|requested| requested == toolset))
                .unwrap_or(false)
        })
        .collect()
}

fn output_schema_for_tool(tool_name: &str) -> Option<Value> {
    match tool_name {
        "run_project_e2e_test" | "run_project_load_test" => Some(execution_start_output_schema()),
        "create_project_e2e_queue" => Some(json!({
            "type": "object",
            "required": ["id", "status"],
            "properties": {
                "id": { "type": "string" },
                "status": { "type": "string" },
                "projectId": { "type": ["string", "null"] }
            }
        })),
        "list_e2e_history" | "list_load_history" => Some(history_list_output_schema()),
        "get_info" => Some(orchestrator_info_output_schema()),
        _ => None,
    }
}

fn execution_start_output_schema() -> Value {
    json!({
        "type": "object",
        "required": ["executionId"],
        "properties": {
            "executionId": { "type": "string" },
            "status": { "type": ["string", "null"] },
            "projectId": { "type": ["string", "null"] }
        }
    })
}

fn history_list_output_schema() -> Value {
    json!({
        "type": "array",
        "items": { "type": "object" }
    })
}

fn orchestrator_info_output_schema() -> Value {
    json!({
        "type": "object",
        "required": ["context", "totalRunners", "activeRunners", "runners"],
        "properties": {
            "context": { "type": "string" },
            "totalRunners": { "type": "integer" },
            "activeRunners": { "type": "integer" },
            "runners": { "type": "array", "items": { "type": "object" } }
        }
    })
}

fn high_risk_tool_name(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "delete_project_pipeline"
            | "delete_project_spec"
            | "import_project"
            | "run_project_load_test"
    )
}

fn add_confirmation_token_property(schema: &mut Value) {
    let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) else {
        return;
    };
    properties.insert(
        "confirmationToken".to_owned(),
        json!({
            "type": ["string", "null"],
            "description": "Required for high-risk calls after the server returns the exact confirmation token."
        }),
    );
}

fn high_risk_tool_reason(tool_name: &str, arguments: &Value) -> Option<String> {
    match tool_name {
        "delete_project_pipeline" => Some("deletes a saved pipeline".to_owned()),
        "delete_project_spec" => Some("deletes a saved API spec".to_owned()),
        "import_project" => Some("imports project data and may create many records".to_owned()),
        "run_project_load_test" => {
            let runner_max_rps = arguments
                .pointer("/load/runnerMaxRps")
                .and_then(Value::as_f64)
                .unwrap_or(600.0);
            let max_intensity = arguments
                .pointer("/load/points")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|point| point.get("intensity").and_then(Value::as_f64))
                .fold(0.0_f64, f64::max);
            if runner_max_rps >= 900.0 || max_intensity >= 90.0 {
                Some("starts a high-intensity load test".to_owned())
            } else {
                None
            }
        }
        _ => None,
    }
}

fn expected_confirmation_token(tool_name: &str, reason: &str) -> String {
    format!("confirm:{tool_name}:{reason}")
}

fn confirmation_token_from_arguments(arguments: &Value) -> Option<&str> {
    arguments.get("confirmationToken").and_then(Value::as_str)
}

fn remove_confirmation_token(arguments: &mut Value) {
    if let Some(object) = arguments.as_object_mut() {
        object.remove("confirmationToken");
    }
}

fn is_supported_mcp_log_level(level: &str) -> bool {
    matches!(level, "debug" | "info" | "notice" | "warning" | "error")
}

fn prompt_definitions() -> Vec<PromptDefinition> {
    vec![
        PromptDefinition {
            name: "default".to_owned(),
            title: Some("Default".to_owned()),
            description: Some(
                "Guides the LLM to create pipelines, evaluate executed tests and steps, and propose safe fixes before applying any change."
                    .to_owned(),
            ),
            arguments: Vec::new(),
        },
        PromptDefinition {
            name: "previa_pipeline_author".to_owned(),
            title: Some("Previa Pipeline Author".to_owned()),
            description: Some(
                "Detailed prompt for creating valid Previa pipelines with schemas, template variables, rules, and examples."
                    .to_owned(),
            ),
            arguments: Vec::new(),
        },
        PromptDefinition {
            name: "local_pipeline_guide".to_owned(),
            title: Some("Local Pipeline Guide".to_owned()),
            description: Some(
                "Explains how to create local pipeline files, import them into a detached Previa context, run them through Previa, and export project pipelines back to local files."
                    .to_owned(),
            ),
            arguments: Vec::new(),
        },
        PromptDefinition {
            name: "project_onboarding_guide".to_owned(),
            title: Some("Project Onboarding Guide".to_owned()),
            description: Some(
                "Guides remote assistants through project discovery, context gathering, and safe next steps before making changes."
                    .to_owned(),
            ),
            arguments: Vec::new(),
        },
        PromptDefinition {
            name: "pipeline_failure_triage".to_owned(),
            title: Some("Pipeline Failure Triage".to_owned()),
            description: Some(
                "Investigates failing E2E and load executions, identifies likely causes, and recommends the next safe action."
                    .to_owned(),
            ),
            arguments: Vec::new(),
        },
        PromptDefinition {
            name: "openapi_spec_ingestion_advisor".to_owned(),
            title: Some("OpenAPI Spec Ingestion Advisor".to_owned()),
            description: Some(
                "Validates OpenAPI content and guides spec creation or updates for a project."
                    .to_owned(),
            ),
            arguments: Vec::new(),
        },
        PromptDefinition {
            name: "pipeline_repair_planner".to_owned(),
            title: Some("Pipeline Repair Planner".to_owned()),
            description: Some(
                "Plans safe, concrete pipeline fixes from execution evidence before any update is applied."
                    .to_owned(),
            ),
            arguments: Vec::new(),
        },
        PromptDefinition {
            name: "load_test_designer".to_owned(),
            title: Some("Load Test Designer".to_owned()),
            description: Some(
                "Designs load test runs with justified parameters, risk notes, and clear execution plans."
                    .to_owned(),
            ),
            arguments: Vec::new(),
        },
        PromptDefinition {
            name: "queue_orchestrator".to_owned(),
            title: Some("Queue Orchestrator".to_owned()),
            description: Some(
                "Helps remote assistants create, monitor, and cancel project E2E queues."
                    .to_owned(),
            ),
            arguments: Vec::new(),
        },
        PromptDefinition {
            name: "http_probe_assistant".to_owned(),
            title: Some("HTTP Probe Assistant".to_owned()),
            description: Some(
                "Uses proxied HTTP requests to inspect live endpoint behavior before proposing persistent pipeline changes."
                    .to_owned(),
            ),
            arguments: Vec::new(),
        },
        PromptDefinition {
            name: "project_migration_assistant".to_owned(),
            title: Some("Project Migration Assistant".to_owned()),
            description: Some(
                "Guides export and import workflows for moving projects between environments."
                    .to_owned(),
            ),
            arguments: Vec::new(),
        },
        PromptDefinition {
            name: "safe_change_reviewer".to_owned(),
            title: Some("Safe Change Reviewer".to_owned()),
            description: Some(
                "Reviews risky create, update, delete, and import actions before they are applied."
                    .to_owned(),
            ),
            arguments: Vec::new(),
        },
        PromptDefinition {
            name: "spec_to_pipeline_bootstrap".to_owned(),
            title: Some("Spec To Pipeline Bootstrap".to_owned()),
            description: Some(
                "Turns project specs into an initial executable pipeline plan with valid assertions and template usage."
                    .to_owned(),
            ),
            arguments: Vec::new(),
        },
    ]
}

fn prompt_result(name: &str) -> Option<PromptGetResult> {
    match name {
        "default" | "pipeline_test_assistant" => Some(prompt_text_result(
            "Operational prompt for pipeline authoring, test analysis, and step repair.",
            pipeline_test_assistant_prompt(),
        )),
        "previa_pipeline_author" | "pipeline_creation_specialist" => Some(prompt_text_result(
            "Detailed prompt for authoring Previa pipelines through MCP.",
            previa_pipeline_author_prompt(),
        )),
        "local_pipeline_guide" => Some(prompt_text_result(
            "Prompt for working with local pipeline files and detached Previa contexts.",
            local_pipeline_guide_prompt(),
        )),
        "project_onboarding_guide" => Some(prompt_text_result(
            "Guided prompt for safely discovering project context before acting.",
            project_onboarding_guide_prompt(),
        )),
        "pipeline_failure_triage" => Some(prompt_text_result(
            "Prompt for investigating failures across E2E and load executions.",
            pipeline_failure_triage_prompt(),
        )),
        "openapi_spec_ingestion_advisor" => Some(prompt_text_result(
            "Prompt for validating and ingesting OpenAPI specs into a project.",
            openapi_spec_ingestion_advisor_prompt(),
        )),
        "pipeline_repair_planner" => Some(prompt_text_result(
            "Prompt for planning evidence-based pipeline repairs before updates.",
            pipeline_repair_planner_prompt(),
        )),
        "load_test_designer" => Some(prompt_text_result(
            "Prompt for designing safe, justified load test executions.",
            load_test_designer_prompt(),
        )),
        "queue_orchestrator" => Some(prompt_text_result(
            "Prompt for operating project E2E queues.",
            queue_orchestrator_prompt(),
        )),
        "http_probe_assistant" => Some(prompt_text_result(
            "Prompt for inspecting live HTTP behavior through proxy requests.",
            http_probe_assistant_prompt(),
        )),
        "project_migration_assistant" => Some(prompt_text_result(
            "Prompt for exporting, reviewing, and importing project bundles.",
            project_migration_assistant_prompt(),
        )),
        "safe_change_reviewer" => Some(prompt_text_result(
            "Prompt for reviewing the impact of risky project changes before execution.",
            safe_change_reviewer_prompt(),
        )),
        "spec_to_pipeline_bootstrap" => Some(prompt_text_result(
            "Prompt for converting project specs into an initial pipeline design.",
            spec_to_pipeline_bootstrap_prompt(),
        )),
        _ => None,
    }
}

fn prompt_text_result(description: &str, text: String) -> PromptGetResult {
    PromptGetResult {
        description: Some(description.to_owned()),
        messages: vec![PromptMessage {
            role: "user".to_owned(),
            content: PromptTextContent { kind: "text", text },
        }],
    }
}

fn tool_success(value: Value) -> ToolCallResult {
    ToolCallResult {
        content: vec![ToolTextContent {
            kind: "text",
            text: serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()),
        }],
        structured_content: Some(value),
        is_error: false,
    }
}

fn tool_error(message: String) -> ToolCallResult {
    ToolCallResult {
        content: vec![ToolTextContent {
            kind: "text",
            text: message,
        }],
        structured_content: None,
        is_error: true,
    }
}

async fn ensure_project_exists(state: &AppState, project_id: &str) -> Result<(), String> {
    if project_exists(&state.db, project_id)
        .await
        .map_err(|err| format!("failed to load project: {err}"))?
    {
        Ok(())
    } else {
        Err(format!("project '{}' not found", project_id))
    }
}

fn queue_tool_outcome(err: QueueError) -> Result<ToolCallResult, String> {
    match err {
        QueueError::BadRequest(message) | QueueError::NotFound(message) => Ok(tool_error(message)),
        QueueError::Internal(message) => Err(message),
    }
}

fn normalize_project_spec_payload(
    mut payload: crate::server::models::ProjectSpecUpsertRequest,
) -> Result<crate::server::models::ProjectSpecUpsertRequest, String> {
    payload.slug = normalize_spec_slug(payload.slug.as_deref())?;
    payload.urls =
        normalize_spec_urls_with_legacy(payload.urls, std::mem::take(&mut payload.servers))?;
    Ok(payload)
}

async fn resolve_project_e2e_request(
    state: &AppState,
    args: RunProjectE2eTestArgs,
) -> Result<E2eTestRequest, String> {
    let (pipeline, pipeline_index) = match (args.pipeline_id.clone(), args.pipeline) {
        (Some(pipeline_id), _) if !pipeline_id.trim().is_empty() => {
            match crate::server::db::load_project_pipeline_for_execution(
                &state.db,
                &args.project_id,
                &pipeline_id,
            )
            .await
            .map_err(|err| format!("failed to load pipeline for execution: {err}"))?
            {
                Some((pipeline, position)) => (pipeline, Some(position)),
                None => return Err("pipelineId not found for project".to_owned()),
            }
        }
        (_, Some(pipeline)) => (pipeline, args.pipeline_index),
        _ => return Err("pipelineId is required".to_owned()),
    };

    Ok(E2eTestRequest {
        pipeline,
        selected_base_url_key: args.selected_base_url_key,
        selected_env_group_slug: args.selected_env_group_slug,
        project_id: Some(args.project_id),
        pipeline_index,
        start_step_id: None,
        prior_results: Default::default(),
        specs: args.specs,
        env_groups: args.env_groups,
    })
}

async fn resolve_project_load_request(
    state: &AppState,
    args: RunProjectLoadTestArgs,
) -> Result<LoadTestRequest, String> {
    let (pipeline, pipeline_index) = match (args.pipeline_id.clone(), args.pipeline) {
        (Some(pipeline_id), _) if !pipeline_id.trim().is_empty() => {
            match crate::server::db::load_project_pipeline_for_execution(
                &state.db,
                &args.project_id,
                &pipeline_id,
            )
            .await
            .map_err(|err| format!("failed to load pipeline for execution: {err}"))?
            {
                Some((pipeline, position)) => (pipeline, Some(position)),
                None => return Err("pipelineId not found for project".to_owned()),
            }
        }
        (_, Some(pipeline)) => (pipeline, args.pipeline_index),
        _ => return Err("pipelineId is required".to_owned()),
    };

    Ok(LoadTestRequest {
        pipeline,
        config: args.config,
        load: args.load,
        target_rps: args.target_rps,
        selected_base_url_key: args.selected_base_url_key,
        selected_env_group_slug: args.selected_env_group_slug,
        project_id: Some(args.project_id),
        pipeline_index,
        specs: args.specs,
        env_groups: args.env_groups,
    })
}

fn execution_start_tool_outcome(err: StartE2eExecutionError) -> Result<ToolCallResult, String> {
    match err {
        StartE2eExecutionError::BadRequest(message)
        | StartE2eExecutionError::ServiceUnavailable(message) => Ok(tool_error(message)),
        StartE2eExecutionError::Internal(message) => Err(message),
    }
}

fn load_execution_start_tool_outcome(
    err: StartLoadExecutionError,
) -> Result<ToolCallResult, String> {
    match err {
        StartLoadExecutionError::BadRequest(message)
        | StartLoadExecutionError::ServiceUnavailable(message) => Ok(tool_error(message)),
        StartLoadExecutionError::Internal(message) => Err(message),
    }
}

async fn execution_started_payload(state: &AppState, execution_id: &str, kind: &str) -> Value {
    let init_payload = {
        let ctx = {
            let executions = state.executions.read().await;
            executions.get(execution_id).cloned()
        };
        match ctx {
            Some(ctx) => ctx.init_payload.get().await,
            None => Value::Null,
        }
    };
    let status = init_payload
        .get("status")
        .and_then(|value| value.as_str())
        .unwrap_or("running");
    json!({
        "executionId": execution_id,
        "status": status,
        "kind": kind,
        "initPayload": init_payload
    })
}

async fn execution_snapshot(
    state: &AppState,
    project_id: &str,
    execution_id: &str,
) -> Result<Option<Value>, String> {
    let active = {
        let executions = state.executions.read().await;
        executions.get(execution_id).cloned()
    };

    if let Some(execution) = active {
        if execution.project_id != project_id {
            return Ok(None);
        }
        let kind = match execution.kind {
            ExecutionKind::E2e => "e2e",
            ExecutionKind::Load => "load",
        };
        let init_payload = execution.init_payload.get().await;
        return Ok(Some(json!({
            "executionId": execution_id,
            "projectId": project_id,
            "active": true,
            "kind": kind,
            "initPayload": init_payload,
        })));
    }

    if let Some(record) = load_e2e_history_record_by_id(&state.db, project_id, execution_id)
        .await
        .map_err(|err| format!("failed to load e2e execution: {err}"))?
    {
        return Ok(Some(json!({
            "executionId": execution_id,
            "projectId": project_id,
            "active": false,
            "kind": "e2e",
            "result": record
        })));
    }

    if let Some(record) = load_load_history_record_by_id(&state.db, project_id, execution_id)
        .await
        .map_err(|err| format!("failed to load load execution: {err}"))?
    {
        return Ok(Some(json!({
            "executionId": execution_id,
            "projectId": project_id,
            "active": false,
            "kind": "load",
            "result": record
        })));
    }

    Ok(None)
}

async fn cancel_execution_payload(
    state: &AppState,
    execution_id: &str,
) -> Result<Option<Value>, String> {
    let execution = {
        let executions = state.executions.read().await;
        executions.get(execution_id).cloned()
    };

    let Some(execution) = execution else {
        return Ok(None);
    };
    let already_cancelled = execution.cancel.is_cancelled();
    execution.cancel.cancel();
    Ok(Some(json!({
        "executionId": execution_id,
        "cancelled": true,
        "alreadyCancelled": already_cancelled,
        "message": if already_cancelled {
            "cancellation already requested"
        } else {
            "cancellation requested"
        }
    })))
}

async fn delete_history_rows(
    db: &DbPool,
    table: &str,
    project_id: &str,
    pipeline_index: Option<i64>,
) -> Result<u64, String> {
    let mut query = format!("DELETE FROM {table} WHERE project_id = ?");
    if pipeline_index.is_some() {
        query.push_str(" AND pipeline_index = ?");
    }
    let mut statement = sqlx::query(&query).bind(project_id);
    if let Some(pipeline_index) = pipeline_index {
        statement = statement.bind(pipeline_index);
    }
    statement
        .execute(db)
        .await
        .map(|result| result.rows_affected())
        .map_err(|err| format!("failed to delete history from {table}: {err}"))
}

fn render_proxy_request(payload: ProxyRequest) -> Result<ProxyRequest, String> {
    let value = serde_json::to_value(payload).map_err(|err| {
        format!(
            "failed to serialize proxy payload for template render: {}",
            err
        )
    })?;
    let rendered = previa_runner::render_template_value_simple(&value);
    serde_json::from_value(rendered)
        .map_err(|err| format!("failed to parse rendered proxy payload: {}", err))
}

async fn proxy_tool_request(
    state: &AppState,
    payload: ProxyRequest,
    max_events: usize,
    timeout_ms: u64,
) -> Result<Value, String> {
    let method = Method::from_bytes(payload.method.trim().as_bytes())
        .map_err(|_| format!("invalid method: {}", payload.method))?;
    let url = payload.url.trim();
    if url.is_empty() {
        return Err("url is required and cannot be empty".to_owned());
    }
    reqwest::Url::parse(url).map_err(|err| format!("invalid url: {}", err))?;

    let mut request = state.client.request(method, url);
    for (name, value) in &payload.headers {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| format!("invalid header name: {}", name))?;
        let header_value = HeaderValue::from_str(value)
            .map_err(|_| format!("invalid header value for {}: {}", name, value))?;
        request = request.header(header_name, header_value);
    }
    if let Some(body) = payload.body {
        request = match body {
            Value::Null => request,
            Value::String(raw) => request.body(raw),
            value => request.json(&value),
        };
    }

    let response = request
        .send()
        .await
        .map_err(|err| format!("proxy request failed: {err}"))?;
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.to_string(), Value::String(v.to_owned())))
        })
        .collect::<serde_json::Map<String, Value>>();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();

    if content_type.contains("text/event-stream") {
        let collected = collect_sse_events(response, max_events, timeout_ms).await?;
        return Ok(json!({
            "status": status,
            "headers": headers,
            "sse": true,
            "events": collected.events,
            "truncated": collected.truncated,
            "timedOut": collected.timed_out
        }));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|err| format!("failed to read upstream response body: {err}"))?;
    let text = String::from_utf8_lossy(&bytes).to_string();
    let json_body = serde_json::from_slice::<Value>(&bytes).ok();
    Ok(json!({
        "status": status,
        "headers": headers,
        "sse": false,
        "bodyText": text,
        "bodyJson": json_body
    }))
}

struct CollectedSseEvents {
    events: Vec<Value>,
    truncated: bool,
    timed_out: bool,
}

async fn collect_sse_events(
    response: reqwest::Response,
    max_events: usize,
    timeout_ms: u64,
) -> Result<CollectedSseEvents, String> {
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut events = Vec::new();
    let mut timed_out = false;

    loop {
        let next = timeout(Duration::from_millis(timeout_ms), stream.next()).await;
        match next {
            Ok(Some(chunk_result)) => {
                let chunk = chunk_result
                    .map_err(|err| format!("failed to read upstream SSE stream: {}", err))?;
                buffer.push_str(&String::from_utf8_lossy(&chunk).replace("\r\n", "\n"));

                while let Some(idx) = buffer.find("\n\n") {
                    let block = buffer[..idx].to_owned();
                    buffer = buffer[idx + 2..].to_owned();
                    if let Some((event, data_text)) = parse_sse_block(&block) {
                        let parsed = serde_json::from_str::<Value>(&data_text)
                            .unwrap_or_else(|_| Value::String(data_text));
                        events.push(json!({ "event": event, "data": parsed }));
                        if events.len() >= max_events {
                            return Ok(CollectedSseEvents {
                                events,
                                truncated: true,
                                timed_out: false,
                            });
                        }
                    }
                }
            }
            Ok(None) => break,
            Err(_) => {
                timed_out = true;
                break;
            }
        }
    }

    if !buffer.trim().is_empty() {
        if let Some((event, data_text)) = parse_sse_block(&buffer) {
            let parsed = serde_json::from_str::<Value>(&data_text)
                .unwrap_or_else(|_| Value::String(data_text));
            events.push(json!({ "event": event, "data": parsed }));
        }
    }

    Ok(CollectedSseEvents {
        events,
        truncated: false,
        timed_out,
    })
}

fn validate_pipeline_input(pipeline: &Pipeline) -> Result<(), String> {
    if pipeline.name.trim().is_empty() {
        return Err("pipeline name is required".to_owned());
    }
    if pipeline.steps.is_empty() {
        return Err("pipeline must contain at least one step".to_owned());
    }
    Ok(())
}

fn pipeline_schema() -> Value {
    json!({
        "type": "object",
        "required": ["name", "steps"],
        "properties": {
            "id": { "type": "string" },
            "name": { "type": "string", "minLength": 1 },
            "description": { "type": ["string", "null"] },
            "steps": {
                "type": "array",
                "minItems": 1,
                "items": pipeline_step_schema()
            }
        }
    })
}

fn pipeline_step_schema() -> Value {
    json!({
        "type": "object",
        "required": ["id", "name", "method", "url"],
        "properties": {
            "id": { "type": "string", "minLength": 1 },
            "name": { "type": "string", "minLength": 1 },
            "description": { "type": ["string", "null"] },
            "method": { "type": "string", "minLength": 1 },
            "url": { "type": "string", "minLength": 1 },
            "headers": {
                "type": "object",
                "additionalProperties": { "type": "string" }
            },
            "body": {},
            "operationId": { "type": ["string", "null"] },
            "delay": { "type": ["integer", "null"], "minimum": 0 },
            "retry": { "type": ["integer", "null"], "minimum": 0 },
            "asserts": {
                "type": "array",
                "items": assertion_schema()
            }
        }
    })
}

fn assertion_schema() -> Value {
    json!({
        "type": "object",
        "required": ["field", "operator"],
        "properties": {
            "field": { "type": "string", "minLength": 1 },
            "operator": { "type": "string", "minLength": 1 },
            "expected": { "type": ["string", "null"] }
        }
    })
}

fn pipeline_creation_guide() -> Value {
    json!({
        "workflow": [
            "1. Call list_projects or get_project to choose the target project.",
            "2. Optionally call list_project_specs to inspect available spec slugs and base URL names for template usage.",
            "3. Build a pipeline object with name, optional description, and at least one step.",
            "4. Use create_project_pipeline with projectId + pipeline.",
            "5. Before execution, templates are validated. Unknown variables like {{run.id}} are rejected."
        ],
        "createTool": "create_project_pipeline",
        "updateTool": "update_project_pipeline",
        "pipelineRules": [
            "pipeline.name is required",
            "pipeline.steps must contain at least one step",
            "each step requires id, name, method, and url",
            "steps.<stepId> references can only target steps that already ran earlier in the same pipeline",
            "specs.<slug>.url.<name> references only work when the project has matching runtime specs configured",
            "supported template locations include step url, headers, body, and assertion expected values"
        ],
        "supportedTemplateVariables": {
            "steps": {
                "pattern": "{{steps.<stepId>.<fieldPath>}}",
                "description": "Reads values from the response body of a previous step.",
                "example": "{{steps.login.token}}"
            },
            "specs": {
                "pattern": "{{specs.<slug>.url.<name>}}",
                "description": "Reads base URLs from runtime specs attached to the project or provided for execution.",
                "example": "{{specs.payments.url.hml}}"
            },
            "helpers": KNOWN_TEMPLATE_HELPERS,
            "helperExamples": [
                "{{helpers.uuid}}",
                "{{helpers.email}}",
                "{{helpers.name}}",
                "{{helpers.username}}",
                "{{helpers.number 1 100}}",
                "{{helpers.date}}",
                "{{helpers.boolean}}",
                "{{helpers.cpf}}"
            ],
            "unsupportedExamples": [
                "{{run.id}}",
                "{{project.id}}",
                "{{pipeline.id}}",
                "{{env.API_URL}}"
            ]
        },
        "exampleCreateProjectPipelineArguments": {
            "projectId": "project_123",
            "pipeline": {
                "name": "Create And Fetch User",
                "description": "Creates a user and then fetches it using the id returned by the first step.",
                "steps": [
                    {
                        "id": "create_user",
                        "name": "Create user",
                        "method": "POST",
                        "url": "{{specs.users.url.hml}}/users",
                        "headers": {
                            "content-type": "application/json",
                            "x-request-id": "{{helpers.uuid}}"
                        },
                        "body": {
                            "name": "{{helpers.name}}",
                            "email": "{{helpers.email}}"
                        },
                        "asserts": [
                            {
                                "field": "status",
                                "operator": "equals",
                                "expected": "201"
                            }
                        ]
                    },
                    {
                        "id": "get_user",
                        "name": "Get user",
                        "method": "GET",
                        "url": "{{specs.users.url.hml}}/users/{{steps.create_user.id}}",
                        "headers": {},
                        "asserts": [
                            {
                                "field": "status",
                                "operator": "equals",
                                "expected": "200"
                            },
                            {
                                "field": "body.email",
                                "operator": "equals",
                                "expected": "{{steps.create_user.email}}"
                            }
                        ]
                    }
                ]
            }
        }
    })
}

fn pipeline_test_assistant_prompt() -> String {
    [
        "You are responsible for operating Previa pipelines through the MCP server.",
        "Your job has three parts: create pipelines, evaluate executed tests and step results, and fix broken steps when needed.",
        "When creating pipelines, prefer this workflow: inspect the project, inspect project specs, call get_pipeline_creation_guide, build the pipeline, then call create_project_pipeline or update_project_pipeline.",
        "Use only supported template variables. Valid roots are steps.<stepId>.*, specs.<slug>.url.<name>, and helpers.*. Do not invent variables such as run.id, project.id, pipeline.id, or env.*.",
        "When evaluating tests, inspect list_e2e_history or list_load_history first, then use get_e2e_test or get_load_test to analyze the exact execution details, including request, response, body, asserts, and step-level failures when available.",
        "When a step fails, identify the most likely root cause from the execution data before suggesting any change. Consider status assertions, request body mistakes, wrong URLs, missing headers, invalid template references, and downstream dependency errors.",
        "When you find a problem, always propose a concrete solution first and then ask the user if they want you to apply it. Do not silently modify a pipeline without explicit user confirmation.",
        "Your proposed solution should be specific. Name the failing step, explain the issue, show the exact change you want to make, and mention which MCP tool you will use to apply it.",
        "If the current data is insufficient to justify a fix, say what is missing and which MCP tool should be called next.",
        "When the user approves a change, update the saved pipeline with update_project_pipeline instead of inventing a non-existent tool name.",
        "When discussing a new pipeline, provide a valid example that matches the input schema accepted by create_project_pipeline.",
    ]
    .join("\n")
}

fn previa_pipeline_author_prompt() -> String {
    let pipeline_schema = serde_json::to_string_pretty(&pipeline_schema()).unwrap();
    let step_schema = serde_json::to_string_pretty(&pipeline_step_schema()).unwrap();
    let assertion_schema = serde_json::to_string_pretty(&assertion_schema()).unwrap();
    let example_payload = serde_json::to_string_pretty(
        &pipeline_creation_guide()["exampleCreateProjectPipelineArguments"],
    )
    .unwrap();

    format!(
        "You are responsible for creating valid Previa pipelines through MCP.\n\
Your goal is to produce payloads that can be sent directly to create_project_pipeline or update_project_pipeline.\n\
\n\
Required workflow:\n\
1. Identify the target project with list_projects or get_project.\n\
2. Inspect the project's runtime specs with list_project_specs before using specs.<slug>.url.<name> variables.\n\
3. Build a pipeline object that matches the schemas below.\n\
4. Prefer explicit status asserts on every HTTP step.\n\
5. Return a final payload compatible with create_project_pipeline.\n\
\n\
Creation rules:\n\
- Always use create_project_pipeline to create a new saved pipeline.\n\
- Always use update_project_pipeline to modify an existing saved pipeline.\n\
- Do not invent non-existent tools such as save_pipeline.\n\
- pipeline.name is required.\n\
- pipeline.steps must contain at least one step.\n\
- Each step requires id, name, method, and url.\n\
- steps.<stepId> references can only point to steps that ran earlier in the same pipeline.\n\
- Supported template locations include url, headers, body, and assertion expected values.\n\
- Unknown variables like {{{{run.id}}}}, {{{{project.id}}}}, {{{{pipeline.id}}}}, and {{{{env.API_URL}}}} are invalid.\n\
\n\
Supported template variables:\n\
- Previous step response body: {{{{steps.<stepId>.<fieldPath>}}}}\n\
  Example: {{{{steps.login.token}}}}\n\
- Runtime spec base URLs: {{{{specs.<slug>.url.<name>}}}}\n\
  Example: {{{{specs.payments.url.hml}}}}\n\
- Helpers:\n\
  - {{{{helpers.uuid}}}}\n\
  - {{{{helpers.email}}}}\n\
  - {{{{helpers.name}}}}\n\
  - {{{{helpers.username}}}}\n\
  - {{{{helpers.number 1 100}}}}\n\
  - {{{{helpers.date}}}}\n\
  - {{{{helpers.boolean}}}}\n\
  - {{{{helpers.cpf}}}}\n\
\n\
Schema for pipeline:\n\
```json\n\
{pipeline_schema}\n\
```\n\
\n\
Schema for step:\n\
```json\n\
{step_schema}\n\
```\n\
\n\
Schema for assertion:\n\
```json\n\
{assertion_schema}\n\
```\n\
\n\
Recommended authoring guidance:\n\
- Use stable, descriptive step ids because later steps depend on them.\n\
- Keep request headers explicit, especially content-type when sending JSON.\n\
- Add status assertions to every step.\n\
- When validating response bodies, use body.<field> assertions.\n\
- When chaining steps, reference values from previous response bodies with steps.<stepId>.<fieldPath>.\n\
- If a pipeline depends on project specs, verify the slug and URL name from list_project_specs before generating the payload.\n\
\n\
Example payload for create_project_pipeline:\n\
```json\n\
{example_payload}\n\
```\n\
\n\
Output requirements:\n\
- Return a valid JSON object for create_project_pipeline arguments.\n\
- Keep the payload directly executable by MCP.\n\
- If required project or spec information is missing, say exactly which MCP tool should be called next instead of guessing.\n"
    )
}

fn local_pipeline_guide_prompt() -> String {
    [
        "You are guiding a user through Previa local pipeline workflows.",
        "Your job is to explain how local pipeline files are created, imported into a detached local context, executed through Previa, and exported back out of a project when needed.",
        "Use only workflows that exist in this repository.",
        "Core local workflow:",
        "1. Create one or more local pipeline files using the same pipeline schema accepted by create_project_pipeline.",
        "2. Save them with one of the supported import extensions: .previa, .previa.json, .previa.yaml, or .previa.yml.",
        "3. Start a detached local context and import those files at startup with previa up --context <context> --detach --import <path> --stack <stack-name>.",
        "4. Use --recursive when <path> is a directory containing multiple pipeline files.",
        "5. After the detached context is running, open Previa with previa open --context <context> or operate it through MCP tools such as list_projects, list_project_pipelines, run_project_e2e_test, and run_project_load_test.",
        "Important local runtime rules:",
        "- --import requires --detach.",
        "- --stack is required when using --import.",
        "- --recursive only works when --import points to a directory.",
        "- Importing local pipelines creates a new project named after the provided stack name.",
        "- If a project with that stack name already exists, import will fail until the conflict is resolved.",
        "Recommended command examples:",
        "- Single file import: previa up --context default --detach --import ./pipelines/user-smoke.previa.yaml --stack local-smoke",
        "- Directory import: previa up --context default --detach --import ./pipelines --recursive --stack local-smoke",
        "- Open the IDE for that context: previa open --context default",
        "- Check the detached context status: previa status --context default",
        "How to export a project into local pipelines:",
        "1. Start or reuse the detached context that can reach the target project.",
        "2. Run previa export pipelines --context <context> --project <project-id-or-name> --output-dir <dir>.",
        "3. Use --pipeline <id-or-name> to export only selected pipelines.",
        "4. Use --format yaml or --format json to choose the local file format.",
        "5. Use --overwrite when the output directory already contains files you intentionally want to replace.",
        "Export examples:",
        "- Export all pipelines: previa export pipelines --context default --project project_123 --output-dir ./pipelines",
        "- Export selected pipeline as JSON: previa export pipelines --context default --project Billing --pipeline smoke-login --format json --output-dir ./pipelines-json",
        "Execution guidance after import:",
        "- Treat imported local files as saved project pipelines inside the running context.",
        "- To execute them programmatically, first discover the imported project with list_projects or get_project, then inspect pipelines with list_project_pipelines, and finally call run_project_e2e_test or run_project_load_test.",
        "- If the user asks to change the local pipeline definition itself, update the local file and re-import it into a fresh project or export the current saved project state back to local files for editing.",
        "Output requirements:",
        "- Prefer concrete commands over abstract descriptions.",
        "- Distinguish clearly between local files on disk, the detached Previa context, and the saved project created during import.",
        "- If the user seems to confuse context and stack, explain that context selects the local runtime while --stack names the imported project created from the local pipeline files.",
    ]
    .join("\n")
}

fn project_onboarding_guide_prompt() -> String {
    [
        "You are onboarding yourself to a Previa project through MCP before making any change.",
        "Your first job is to build a concise mental model of the project and expose it to the user.",
        "Recommended workflow:",
        "1. Call list_projects or get_project to identify the target project.",
        "2. Call list_project_specs to discover available specs, slugs, and base URL names.",
        "3. Call list_project_pipelines to understand what is already automated.",
        "4. If the user mentions failures or runs, inspect list_e2e_history or list_load_history before proposing edits.",
        "5. Summarize the current state, open risks, and the best next MCP action.",
        "Output requirements:",
        "- Return a short onboarding summary with project purpose, known specs, existing pipelines, and obvious gaps.",
        "- Distinguish facts gathered from MCP from assumptions that still need confirmation.",
        "- Do not propose create, update, delete, or import actions until you have shown the current context.",
        "- If critical context is missing, name the exact MCP tool that should be called next.",
    ]
    .join("\n")
}

fn pipeline_failure_triage_prompt() -> String {
    [
        "You are triaging a failed Previa execution through MCP.",
        "Always begin with evidence, not guesses.",
        "Required workflow:",
        "1. Use list_e2e_history or list_load_history to identify the relevant failure when the execution is not already known.",
        "2. Use get_e2e_test, get_load_test, or get_execution to inspect the exact request, response, asserts, and failed steps.",
        "3. Identify the most likely root cause and explain why it is more plausible than nearby alternatives.",
        "4. Recommend the next safe action: observe more data, probe the endpoint, adjust a pipeline, or rerun after confirmation.",
        "Output requirements:",
        "- Name the failing step or execution segment.",
        "- Separate observed evidence, likely cause, confidence level, and next action.",
        "- When you recommend a pipeline change, mention update_project_pipeline but do not apply it without user approval.",
        "- If the failure points to live API behavior, propose proxy_request before editing the pipeline.",
    ]
    .join("\n")
}

fn openapi_spec_ingestion_advisor_prompt() -> String {
    [
        "You are responsible for safely ingesting OpenAPI content into a Previa project through MCP.",
        "Your goal is to validate the source, explain issues clearly, and produce the correct create or update action.",
        "Required workflow:",
        "1. Validate the provided source with validate_openapi before proposing persistence.",
        "2. Inspect the target project with get_project and list_project_specs.",
        "3. Decide whether the operation should use create_project_spec or update_project_spec.",
        "4. Verify slug, URL names, and any project-specific conventions before proposing a final payload.",
        "Output requirements:",
        "- Report validation findings first.",
        "- Call out schema errors, missing servers, ambiguous slugs, and naming conflicts explicitly.",
        "- Return a payload compatible with create_project_spec or update_project_spec only when the source is valid enough.",
        "- If the source is not ready, explain what must change before the MCP write call should happen.",
    ]
    .join("\n")
}

fn pipeline_repair_planner_prompt() -> String {
    [
        "You are planning a safe repair for an existing Previa pipeline.",
        "Use execution data and current pipeline state together before proposing a fix.",
        "Required workflow:",
        "1. Inspect the failing execution with get_e2e_test, get_load_test, or get_execution.",
        "2. Fetch the saved pipeline with get_project_pipeline.",
        "3. Compare observed failure data with the current step definitions, template references, URLs, headers, and assertions.",
        "4. Propose the smallest effective patch that resolves the evidence-backed issue.",
        "Output requirements:",
        "- Name the exact step to change.",
        "- Show the before-and-after intent in plain language.",
        "- Mention update_project_pipeline as the write tool, but wait for explicit approval before applying it.",
        "- If the evidence is weak, ask for one more diagnostic MCP call instead of overfitting the fix.",
    ]
    .join("\n")
}

fn load_test_designer_prompt() -> String {
    [
        "You are designing Previa load tests through MCP.",
        "Your job is to choose realistic wave load profiles and explain why they fit the user's goal.",
        "Required workflow:",
        "1. Confirm the target project and pipeline with get_project, list_project_pipelines, or get_project_pipeline.",
        "2. If needed, inspect prior load results with list_load_history and get_load_test.",
        "3. Prefer a wave load payload with points as { atMs, intensity }, where intensity is 0-100 percent of runnerMaxRps for each active runner.",
        "4. Use duration presets when they fit: 1m (60000ms), 10m (600000ms), 30m (1800000ms), or custom atMs values for specific experiments.",
        "5. Set load.runnerMaxRps between 1 and 1000 when the test needs an explicit per-runner cap; omit it to use the default of 600.",
        "6. Use smooth interpolation by default. Use step only for explicit spike/degradation tests.",
        "7. Treat the request count shown at each wave point as the maximum scheduled request rate at that point: active runners * runnerMaxRps * intensity / 100.",
        "8. Explain gracePeriodMs as extra time to observe pending responses after scheduling stops; the run can finish sooner when all responses have been observed.",
        "9. Highlight operational risks such as overly high intensity, missing assertions, unstable environments, or slow responses creating a large pending-response backlog.",
        "Output requirements:",
        "- Present a runnable payload for run_project_load_test when enough context exists.",
        "- Explain what the run is trying to learn.",
        "- Distinguish smoke, baseline, and stress-style configurations when helpful.",
        "- If the underlying pipeline looks weak, recommend fixing the pipeline before scaling load.",
    ]
    .join("\n")
}

fn queue_orchestrator_prompt() -> String {
    [
        "You are operating Previa E2E queues for a remote user through MCP.",
        "Your job is to sequence pipelines clearly, track queue state, and avoid surprise actions.",
        "Required workflow:",
        "1. Inspect available pipelines with list_project_pipelines.",
        "2. Use create_project_e2e_queue only after confirming the intended pipeline order and base URL selection.",
        "3. Use get_current_project_e2e_queue or get_project_e2e_queue to explain progress and current status.",
        "4. Use cancel_project_e2e_queue only when the user requests cancellation or when you are explicitly asked what the cancel path is.",
        "Output requirements:",
        "- Summarize queue composition, active item, completed items, failures, and remaining work.",
        "- Make it obvious whether the queue is running, completed, or canceled.",
        "- Do not invent queue tools or background monitoring features that do not exist.",
    ]
    .join("\n")
}

fn http_probe_assistant_prompt() -> String {
    [
        "You are inspecting live HTTP behavior before changing saved Previa assets.",
        "Use proxy_request to gather real evidence from endpoints, auth flows, headers, payloads, redirects, and SSE streams.",
        "Required workflow:",
        "1. Define the smallest probe that can answer the user's question.",
        "2. Use proxy_request with explicit request details and bounded maxEvents or timeoutMs when probing SSE.",
        "3. Report status code, headers, body shape, and any mismatches with current pipeline expectations.",
        "4. Recommend whether the next step should be another probe, a pipeline update, a spec update, or no change.",
        "Output requirements:",
        "- Keep the probe purpose clear and narrow.",
        "- Call out sensitive headers or auth assumptions when relevant.",
        "- Treat proxy evidence as a diagnostic input, not an automatic justification to edit saved pipelines.",
    ]
    .join("\n")
}

fn project_migration_assistant_prompt() -> String {
    [
        "You are guiding a project migration through Previa MCP export and import tools.",
        "Your job is to move data carefully between environments and explain what is included.",
        "Required workflow:",
        "1. Inspect the source project with get_project, list_project_pipelines, and list_project_specs when needed.",
        "2. Use export_project to create a bundle and decide deliberately whether includeHistory should be true.",
        "3. Review the bundle contents at a high level before import_project.",
        "4. After import, verify the destination state with get_project, list_project_pipelines, and list_project_specs.",
        "Output requirements:",
        "- Explain what will move: metadata, specs, pipelines, and optionally history.",
        "- Call out overwrite or duplication risks before import.",
        "- If the user asks for a migration plan only, do not execute import_project automatically.",
    ]
    .join("\n")
}

fn safe_change_reviewer_prompt() -> String {
    [
        "You are reviewing potentially risky MCP write actions before they are executed.",
        "You should make the blast radius visible and keep remote changes deliberate.",
        "Review scope includes create_project, update_project, delete_project, import_project, create_project_pipeline, update_project_pipeline, delete_project_pipeline, create_project_spec, update_project_spec, and delete_project_spec.",
        "Output requirements:",
        "- Summarize the intended action, affected resources, likely impact, and rollback path.",
        "- Highlight destructive or irreversible consequences explicitly.",
        "- When the action depends on assumptions, list them before recommending execution.",
        "- Ask for explicit confirmation before delete and import flows or any broad update with uncertain impact.",
    ]
    .join("\n")
}

fn spec_to_pipeline_bootstrap_prompt() -> String {
    [
        "You are turning existing project specs into an initial Previa pipeline design.",
        "Your goal is to bootstrap a useful saved pipeline from the API contract without inventing unsupported variables or tools.",
        "Required workflow:",
        "1. Inspect the target project with get_project and list_project_specs.",
        "2. Use get_project_spec and get_pipeline_creation_guide to understand the available base URLs, schema hints, and supported template variables.",
        "3. Build a practical first pipeline with explicit status assertions and stable step ids.",
        "4. Return a payload compatible with create_project_pipeline.",
        "Output requirements:",
        "- Prefer a narrow smoke-style flow over an overly ambitious end-to-end journey.",
        "- Use specs.<slug>.url.<name> only after verifying the exact slug and URL name.",
        "- If the spec is too incomplete to bootstrap safely, explain the gap before generating a payload.",
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;

    use previa_runner::{Pipeline, PipelineStep};
    use serde_json::json;
    use tokio::sync::RwLock;

    use super::{
        execute_tool, filter_completion_values, filter_tools_by_toolset, high_risk_tool_reason,
        is_supported_mcp_log_level, load_test_designer_prompt, local_pipeline_guide_prompt,
        orchestrator_info_resource_uri, parse_tool_arguments, pipeline_creation_guide,
        pipeline_test_assistant_prompt, previa_pipeline_author_prompt,
        project_current_e2e_queue_resource_uri, project_e2e_history_resource_uri,
        project_load_history_resource_uri, prompt_definitions, prompt_result,
        resource_template_definitions, runners_resource_uri, tool_definition, tool_definitions,
        validate_pipeline_input,
    };
    use crate::server::db::{
        insert_project_pipeline, load_e2e_queue_record, upsert_project_metadata,
    };
    use crate::server::execution::ExecutionScheduler;
    use crate::server::mcp::models::{
        CreateProjectArgs, CreateProjectE2eQueueArgs, CreateProjectPipelineArgs, ProjectByIdArgs,
        ProjectHistoryToolArgs, ToolCallParams,
    };
    use crate::server::models::ProjectMetadataUpsertRequest;
    use crate::server::state::AppState;

    #[test]
    fn project_tools_require_project_id() {
        let tool = tool_definitions()
            .into_iter()
            .find(|tool| tool.name == "get_project")
            .expect("get_project tool definition");

        assert_eq!(tool.input_schema["required"], json!(["projectId"]));
    }

    #[test]
    fn parse_project_argument_payload() {
        let args = parse_tool_arguments::<ProjectByIdArgs>(json!({ "projectId": "abc" }))
            .expect("valid project args");

        assert_eq!(args.project_id, "abc");
    }

    #[test]
    fn parse_project_argument_payload_with_meta() {
        let args = parse_tool_arguments::<ProjectByIdArgs>(
            json!({ "projectId": "abc", "_meta": { "source": "client" } }),
        )
        .expect("valid project args with meta");

        assert_eq!(args.project_id, "abc");
        assert_eq!(args.meta, Some(json!({ "source": "client" })));
    }

    #[test]
    fn parse_create_pipeline_arguments() {
        let args = parse_tool_arguments::<CreateProjectPipelineArgs>(json!({
            "projectId": "project-1",
            "pipeline": {
                "name": "Pipeline A",
                "description": null,
                "steps": [
                    {
                        "id": "step-1",
                        "name": "Step 1",
                        "method": "GET",
                        "url": "https://example.com",
                        "headers": {},
                        "asserts": []
                    }
                ]
            }
        }))
        .expect("valid create pipeline args");

        assert_eq!(args.project_id, "project-1");
        assert_eq!(args.pipeline.name, "Pipeline A");
    }

    #[test]
    fn parse_create_e2e_queue_arguments() {
        let args = parse_tool_arguments::<CreateProjectE2eQueueArgs>(json!({
            "projectId": "project-1",
            "pipelineIds": ["pipeline-1", "pipeline-2"],
            "selectedBaseUrlKey": "hml",
            "specs": []
        }))
        .expect("valid create e2e queue args");

        assert_eq!(args.project_id, "project-1");
        assert_eq!(args.pipeline_ids, vec!["pipeline-1", "pipeline-2"]);
        assert_eq!(args.selected_base_url_key.as_deref(), Some("hml"));
    }

    #[test]
    fn parse_create_project_arguments() {
        let args = parse_tool_arguments::<CreateProjectArgs>(json!({
            "project": {
                "name": "Project A",
                "description": "desc",
                "pipelines": []
            }
        }))
        .expect("valid create project args");

        assert_eq!(args.project.name, "Project A");
    }

    #[test]
    fn validate_pipeline_requires_name() {
        let pipeline = Pipeline {
            id: None,
            name: "   ".to_owned(),
            description: None,
            steps: vec![PipelineStep {
                id: "step-1".to_owned(),
                name: "Step 1".to_owned(),
                description: None,
                method: "GET".to_owned(),
                url: "https://example.com".to_owned(),
                headers: Default::default(),
                body: None,
                operation_id: None,
                delay: None,
                retry: None,
                asserts: Vec::new(),
            }],
        };

        assert_eq!(
            validate_pipeline_input(&pipeline).expect_err("pipeline name should be validated"),
            "pipeline name is required"
        );
    }

    #[test]
    fn pipeline_guide_tool_is_available() {
        let tool = tool_definitions()
            .into_iter()
            .find(|tool| tool.name == "get_pipeline_creation_guide")
            .expect("pipeline guide tool definition");

        assert_eq!(tool.input_schema["type"], json!("object"));
    }

    #[test]
    fn e2e_queue_tools_are_available() {
        let tools = tool_definitions();

        for name in [
            "create_project",
            "update_project",
            "delete_project",
            "export_project",
            "import_project",
            "get_project_spec",
            "create_project_spec",
            "update_project_spec",
            "delete_project_spec",
            "create_project_e2e_queue",
            "get_current_project_e2e_queue",
            "get_project_e2e_queue",
            "cancel_project_e2e_queue",
            "run_project_e2e_test",
            "run_project_load_test",
            "get_execution",
            "cancel_execution",
            "delete_e2e_history",
            "delete_e2e_test",
            "delete_load_history",
            "delete_load_test",
            "proxy_request",
        ] {
            assert!(
                tools.iter().any(|tool| tool.name == name),
                "missing MCP tool definition for {name}"
            );
        }
    }

    #[test]
    fn pipeline_guide_mentions_unsupported_run_id() {
        let guide = pipeline_creation_guide();

        assert!(
            guide["supportedTemplateVariables"]["unsupportedExamples"]
                .as_array()
                .expect("unsupported examples array")
                .iter()
                .any(|value| value == "{{run.id}}")
        );
    }

    #[test]
    fn pipeline_prompt_is_available() {
        let prompt = prompt_definitions()
            .into_iter()
            .find(|prompt| prompt.name == "default")
            .expect("pipeline prompt definition");

        assert_eq!(prompt.arguments.len(), 0);
    }

    #[test]
    fn pipeline_prompt_mentions_pipeline_creation_tool_and_confirmation() {
        let prompt = prompt_result("default").expect("pipeline prompt");
        let text = &prompt.messages[0].content.text;

        assert!(text.contains("create_project_pipeline"));
        assert!(text.contains("ask the user if they want you to apply it"));
    }

    #[test]
    fn legacy_pipeline_prompt_alias_is_still_available() {
        let prompt = prompt_result("pipeline_test_assistant").expect("legacy pipeline prompt");

        assert!(
            prompt.messages[0]
                .content
                .text
                .contains("create_project_pipeline")
        );
    }

    #[test]
    fn pipeline_prompt_mentions_execution_analysis_tools() {
        let text = pipeline_test_assistant_prompt();

        assert!(text.contains("get_e2e_test"));
        assert!(text.contains("get_load_test"));
        assert!(text.contains("update_project_pipeline"));
    }

    #[test]
    fn pipeline_creation_prompt_is_available() {
        let prompt = prompt_definitions()
            .into_iter()
            .find(|prompt| prompt.name == "previa_pipeline_author")
            .expect("pipeline creation prompt definition");

        assert_eq!(prompt.arguments.len(), 0);
    }

    #[test]
    fn pipeline_creation_prompt_mentions_schema_variables_and_examples() {
        let text = previa_pipeline_author_prompt();

        assert!(text.contains("Schema for pipeline"));
        assert!(text.contains("{{steps.<stepId>.<fieldPath>}}"));
        assert!(text.contains("{{specs.<slug>.url.<name>}}"));
        assert!(text.contains("Example payload for create_project_pipeline"));
        assert!(text.contains("save_pipeline"));
    }

    #[test]
    fn pipeline_creation_prompt_result_is_available() {
        let prompt =
            prompt_result("previa_pipeline_author").expect("pipeline creation prompt result");

        assert!(
            prompt.messages[0]
                .content
                .text
                .contains("create_project_pipeline")
        );
    }

    #[test]
    fn local_pipeline_guide_prompt_is_available() {
        let prompt = prompt_definitions()
            .into_iter()
            .find(|prompt| prompt.name == "local_pipeline_guide")
            .expect("local pipeline guide prompt definition");

        assert_eq!(prompt.arguments.len(), 0);
    }

    #[test]
    fn local_pipeline_guide_mentions_import_export_and_execution_commands() {
        let text = local_pipeline_guide_prompt();

        assert!(text.contains(
            "previa up --context <context> --detach --import <path> --stack <stack-name>"
        ));
        assert!(text.contains(
            "previa export pipelines --context <context> --project <project-id-or-name> --output-dir <dir>"
        ));
        assert!(text.contains("run_project_e2e_test"));
        assert!(text.contains("run_project_load_test"));
        assert!(text.contains(".previa.yaml"));
    }

    #[test]
    fn legacy_pipeline_creation_prompt_alias_is_still_available() {
        let prompt = prompt_result("pipeline_creation_specialist")
            .expect("legacy pipeline creation prompt result");

        assert!(
            prompt.messages[0]
                .content
                .text
                .contains("create_project_pipeline")
        );
    }

    #[test]
    fn remote_assistant_prompts_are_available() {
        let prompts = prompt_definitions();

        for name in [
            "project_onboarding_guide",
            "pipeline_failure_triage",
            "openapi_spec_ingestion_advisor",
            "pipeline_repair_planner",
            "local_pipeline_guide",
            "load_test_designer",
            "queue_orchestrator",
            "http_probe_assistant",
            "project_migration_assistant",
            "safe_change_reviewer",
            "spec_to_pipeline_bootstrap",
        ] {
            assert!(
                prompts.iter().any(|prompt| prompt.name == name),
                "missing MCP prompt definition for {name}"
            );
        }
    }

    #[test]
    fn remote_assistant_prompts_reference_expected_tools() {
        for (name, expected) in [
            ("project_onboarding_guide", "list_project_pipelines"),
            ("pipeline_failure_triage", "get_e2e_test"),
            ("openapi_spec_ingestion_advisor", "validate_openapi"),
            ("pipeline_repair_planner", "get_project_pipeline"),
            ("local_pipeline_guide", "previa export pipelines"),
            ("load_test_designer", "run_project_load_test"),
            ("queue_orchestrator", "create_project_e2e_queue"),
            ("http_probe_assistant", "proxy_request"),
            ("project_migration_assistant", "export_project"),
            ("safe_change_reviewer", "delete_project_pipeline"),
            ("spec_to_pipeline_bootstrap", "get_pipeline_creation_guide"),
        ] {
            let prompt = prompt_result(name).expect("prompt result");
            assert!(
                prompt.messages[0].content.text.contains(expected),
                "prompt {name} should mention {expected}"
            );
        }
    }

    #[test]
    fn parse_history_arguments() {
        let args = parse_tool_arguments::<ProjectHistoryToolArgs>(json!({
            "projectId": "project-1",
            "pipelineIndex": 2,
            "limit": 50,
            "offset": 0,
            "order": "desc"
        }))
        .expect("valid history args");

        assert_eq!(args.project_id, "project-1");
        assert_eq!(args.pipeline_index, Some(2));
        assert_eq!(args.limit, Some(50));
    }

    #[test]
    fn project_history_resource_uris_are_stable() {
        assert_eq!(
            project_e2e_history_resource_uri("project-1"),
            "previa://projects/project-1/history/e2e"
        );
        assert_eq!(
            project_load_history_resource_uri("project-1"),
            "previa://projects/project-1/history/load"
        );
    }

    #[test]
    fn operational_resource_uris_are_stable() {
        assert_eq!(
            orchestrator_info_resource_uri(),
            "previa://orchestrator/info"
        );
        assert_eq!(runners_resource_uri(), "previa://runners");
        assert_eq!(
            project_current_e2e_queue_resource_uri("project-1"),
            "previa://projects/project-1/queues/e2e/current"
        );
    }

    #[test]
    fn resource_templates_include_dynamic_project_paths() {
        let templates = resource_template_definitions();
        let uris = templates
            .iter()
            .map(|template| template.uri_template.as_str())
            .collect::<Vec<_>>();

        assert!(uris.contains(&"previa://projects/{projectId}"));
        assert!(uris.contains(&"previa://projects/{projectId}/pipelines/{pipelineRef}"));
        assert!(uris.contains(&"previa://projects/{projectId}/history/load/{testId}"));
    }

    #[test]
    fn completion_filter_matches_prefix_and_limits_results() {
        let values = (0..120).map(|index| format!("project-{index:03}"));

        let filtered = filter_completion_values(values, "project-0");

        assert_eq!(filtered.len(), 100);
        assert_eq!(filtered[0], "project-000");
        assert_eq!(filtered[99], "project-099");
    }

    #[test]
    fn toolset_filter_returns_only_requested_group() {
        let tools = vec![
            tool_definition("a", "A", "A tool", json!({}), None, "core"),
            tool_definition("b", "B", "B tool", json!({}), None, "load"),
        ];

        let filtered = filter_tools_by_toolset(tools, &[String::from("load")]);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "b");
    }

    #[test]
    fn load_test_tool_schema_exposes_runner_max_rps() {
        let tool = tool_definitions()
            .into_iter()
            .find(|tool| tool.name == "run_project_load_test")
            .expect("run_project_load_test tool definition");

        let runner_max_rps = &tool.input_schema["properties"]["load"]["properties"]["runnerMaxRps"];
        assert_eq!(runner_max_rps["minimum"], json!(1));
        assert_eq!(runner_max_rps["maximum"], json!(1000));
    }

    #[test]
    fn execution_tools_have_output_schemas() {
        let tools = tool_definitions();
        for name in ["run_project_e2e_test", "run_project_load_test"] {
            let tool = tools
                .iter()
                .find(|tool| tool.name == name)
                .expect("execution tool");
            assert!(
                tool.output_schema.is_some(),
                "{name} should expose outputSchema"
            );
        }
    }

    #[test]
    fn mcp_log_level_validation_is_strict() {
        assert!(is_supported_mcp_log_level("info"));
        assert!(is_supported_mcp_log_level("warning"));
        assert!(!is_supported_mcp_log_level("verbose"));
    }

    #[test]
    fn high_risk_load_test_requires_confirmation_at_high_intensity() {
        let reason = high_risk_tool_reason(
            "run_project_load_test",
            &json!({
                "load": {
                    "runnerMaxRps": 950,
                    "points": [
                        { "atMs": 0, "intensity": 0 },
                        { "atMs": 60000, "intensity": 100 }
                    ]
                }
            }),
        );

        assert_eq!(reason, Some("starts a high-intensity load test".to_owned()));
    }

    #[test]
    fn normal_load_test_does_not_require_confirmation() {
        let reason = high_risk_tool_reason(
            "run_project_load_test",
            &json!({
                "load": {
                    "runnerMaxRps": 600,
                    "points": [
                        { "atMs": 0, "intensity": 0 },
                        { "atMs": 60000, "intensity": 60 }
                    ]
                }
            }),
        );

        assert_eq!(reason, None);
    }

    #[test]
    fn load_test_prompt_mentions_current_load_controls() {
        let text = load_test_designer_prompt();

        assert!(text.contains("1m"));
        assert!(text.contains("10m"));
        assert!(text.contains("30m"));
        assert!(text.contains("runnerMaxRps"));
        assert!(text.contains("default of 600"));
        assert!(text.contains("gracePeriodMs"));
    }

    #[tokio::test]
    async fn create_and_get_project_e2e_queue_tools_return_snapshots() {
        let state = test_state().await;
        seed_project_with_pipeline(&state, "project-1", "pipeline-1").await;

        let created = execute_tool(
            &state,
            ToolCallParams {
                name: "create_project_e2e_queue".to_owned(),
                arguments: json!({
                    "projectId": "project-1",
                    "pipelineIds": ["pipeline-1"]
                }),
                meta: None,
            },
        )
        .await
        .expect("create queue tool result");

        assert!(!created.is_error);
        let queue = created
            .structured_content
            .expect("create queue structured content");
        let queue_id = queue["id"].as_str().expect("queue id").to_owned();
        assert_eq!(queue["status"], json!("pending"));

        let terminal = wait_for_terminal_queue(&state, "project-1", &queue_id).await;
        assert_eq!(terminal["id"], json!(queue_id.clone()));
        assert!(matches!(
            terminal["status"].as_str(),
            Some("failed" | "completed" | "cancelled")
        ));

        let loaded = execute_tool(
            &state,
            ToolCallParams {
                name: "get_project_e2e_queue".to_owned(),
                arguments: json!({
                    "projectId": "project-1",
                    "queueId": queue_id
                }),
                meta: None,
            },
        )
        .await
        .expect("get queue tool result");

        assert!(!loaded.is_error);
        assert_eq!(loaded.structured_content.expect("queue snapshot"), terminal);
    }

    #[tokio::test]
    async fn get_current_project_e2e_queue_returns_error_without_active_queue() {
        let state = test_state().await;
        seed_project_with_pipeline(&state, "project-1", "pipeline-1").await;

        let result = execute_tool(
            &state,
            ToolCallParams {
                name: "get_current_project_e2e_queue".to_owned(),
                arguments: json!({ "projectId": "project-1" }),
                meta: None,
            },
        )
        .await
        .expect("current queue tool result");

        assert!(result.is_error);
        assert!(result.structured_content.is_none());
        assert_eq!(
            result.content[0].text,
            "no active e2e queue for project".to_owned()
        );
    }

    #[tokio::test]
    async fn project_and_spec_crud_tools_work() {
        let state = test_state().await;

        let created = execute_tool(
            &state,
            ToolCallParams {
                name: "create_project".to_owned(),
                arguments: json!({
                    "project": {
                        "name": "Project A",
                        "description": "desc",
                        "pipelines": []
                    }
                }),
                meta: None,
            },
        )
        .await
        .expect("create project");
        assert!(!created.is_error);
        let project = created.structured_content.expect("project body");
        let project_id = project["id"].as_str().expect("project id").to_owned();

        let created_spec = execute_tool(
            &state,
            ToolCallParams {
                name: "create_project_spec".to_owned(),
                arguments: json!({
                    "projectId": project_id,
                    "spec": {
                        "spec": {
                            "openapi": "3.0.0",
                            "info": { "title": "API", "version": "1.0.0" },
                            "paths": {}
                        },
                        "slug": "users",
                        "urls": [{"name":"hml","url":"https://example.com"}],
                        "sync": false,
                        "live": false
                    }
                }),
                meta: None,
            },
        )
        .await
        .expect("create spec");
        assert!(!created_spec.is_error);
        let spec = created_spec.structured_content.expect("spec body");
        let spec_id = spec["id"].as_str().expect("spec id").to_owned();

        let deleted_spec = execute_tool(
            &state,
            ToolCallParams {
                name: "delete_project_spec".to_owned(),
                arguments: json!({
                    "projectId": project["id"],
                    "specId": spec_id
                }),
                meta: None,
            },
        )
        .await
        .expect("delete spec");
        assert!(!deleted_spec.is_error);

        let deleted_project = execute_tool(
            &state,
            ToolCallParams {
                name: "delete_project".to_owned(),
                arguments: json!({
                    "projectId": project["id"]
                }),
                meta: None,
            },
        )
        .await
        .expect("delete project");
        assert!(!deleted_project.is_error);
    }

    async fn test_state() -> AppState {
        let db = crate::server::db::DbPool::connect("sqlite::memory:", 1)
            .await
            .expect("sqlite memory db");
        sqlx::migrate!("./migrations/sqlite")
            .run(db.pool())
            .await
            .expect("migrations");

        AppState {
            client: reqwest::Client::new(),
            db,
            context_name: "test".to_owned(),
            runner_auth_key: None,
            auth: crate::server::auth::AuthRuntime::anonymous(),
            rps_per_node: 1,
            scheduler: ExecutionScheduler::new(Default::default()),
            executions: Arc::new(RwLock::new(HashMap::new())),
            e2e_queues: Arc::new(RwLock::new(HashMap::new())),
            mcp_sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn seed_project_with_pipeline(state: &AppState, project_id: &str, pipeline_id: &str) {
        upsert_project_metadata(
            &state.db,
            project_id.to_owned(),
            ProjectMetadataUpsertRequest {
                name: "Project".to_owned(),
                description: Some("Queue test project".to_owned()),
                tags: Vec::new(),
            },
        )
        .await
        .expect("project upsert");

        insert_project_pipeline(
            &state.db,
            project_id,
            Pipeline {
                id: Some(pipeline_id.to_owned()),
                name: "Pipeline".to_owned(),
                description: Some("Queue test pipeline".to_owned()),
                steps: vec![PipelineStep {
                    id: "step-1".to_owned(),
                    name: "Step 1".to_owned(),
                    description: None,
                    method: "GET".to_owned(),
                    url: "https://example.com".to_owned(),
                    headers: Default::default(),
                    body: None,
                    operation_id: None,
                    delay: None,
                    retry: None,
                    asserts: Vec::new(),
                }],
            },
        )
        .await
        .expect("pipeline insert");
    }

    async fn wait_for_terminal_queue(
        state: &AppState,
        project_id: &str,
        queue_id: &str,
    ) -> serde_json::Value {
        for _ in 0..20 {
            let snapshot = load_e2e_queue_record(&state.db, project_id, queue_id)
                .await
                .expect("queue load")
                .expect("queue exists");
            let value = serde_json::to_value(snapshot).expect("queue to value");
            if matches!(
                value["status"].as_str(),
                Some("failed" | "completed" | "cancelled")
            ) {
                return value;
            }

            tokio::time::sleep(Duration::from_millis(25)).await;
        }

        panic!("queue {queue_id} did not reach a terminal state in time");
    }
}
