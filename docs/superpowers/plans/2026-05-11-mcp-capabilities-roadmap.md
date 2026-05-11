# MCP Capabilities Roadmap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Upgrade Previa's MCP server from a useful tool surface into a modern, discoverable, safer agent platform.

**Architecture:** Keep MCP transport and protocol dispatch in `main/src/server/mcp/service.rs`, keep protocol DTOs in `main/src/server/mcp/models.rs`, and reuse existing DB/service functions instead of duplicating application logic. Each plan below can ship independently and should preserve backward compatibility for current MCP clients.

**Tech Stack:** Rust, Axum JSON-RPC handlers, Serde, SQLx-backed project/history services, MCP Streamable HTTP.

---

## Plan 1: Resource Templates And Completions

**Goal:** Let MCP clients discover dynamic Previa URI shapes and autocomplete project, pipeline, spec, and test identifiers.

**Architecture:** Add `resources/templates/list` and `completion/complete` handlers beside the existing `resources/list` and `resources/read` handlers. Resource templates describe URI shapes; completions query existing project, pipeline, spec, and history records using current DB helpers.

**Files:**
- Modify: `main/src/server/mcp/models.rs`
- Modify: `main/src/server/mcp/service.rs`
- Test: `main/src/server/mcp/service.rs`

### Task 1: Add MCP DTOs For Resource Templates And Completion

- [ ] **Step 1: Add model structs**

In `main/src/server/mcp/models.rs`, add these structs near `ResourceDefinition`:

```rust
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceTemplateDefinition {
    pub uri_template: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceTemplatesListParams {
    pub cursor: Option<String>,
    #[serde(default, rename = "_meta")]
    pub meta: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum CompletionReference {
    #[serde(rename = "ref/resource")]
    Resource { uri: String },
    #[serde(rename = "ref/prompt")]
    Prompt { name: String },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompletionArgument {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompletionContext {
    #[serde(default)]
    pub arguments: serde_json::Map<String, Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompletionCompleteParams {
    #[serde(rename = "ref")]
    pub reference: CompletionReference,
    pub argument: CompletionArgument,
    #[serde(default)]
    pub context: CompletionContext,
    #[serde(default, rename = "_meta")]
    pub meta: Option<Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionResult {
    pub completion: CompletionValues,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionValues {
    pub values: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<usize>,
    pub has_more: bool,
}
```

- [ ] **Step 2: Import the new DTOs in the MCP service**

In `main/src/server/mcp/service.rs`, extend the `crate::server::mcp::models` import with:

```rust
CompletionCompleteParams, CompletionReference, CompletionResult, CompletionValues,
ResourceTemplateDefinition, ResourceTemplatesListParams,
```

- [ ] **Step 3: Run formatting**

Run: `cargo fmt`

Expected: command exits with code `0`.

### Task 2: Expose Resource Templates

- [ ] **Step 1: Add template definitions**

In `main/src/server/mcp/service.rs`, add this function near `list_resources`:

```rust
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
```

- [ ] **Step 2: Advertise resource template support**

In `handle_initialize`, update the `resources` capability:

```rust
"resources": {
    "listChanged": false,
    "subscribe": false
},
"completions": {},
```

Do not remove the existing `resources` capability; add `completions` as a sibling capability.

- [ ] **Step 3: Handle `resources/templates/list`**

In `process_request`, add a match arm beside `resources/list`:

```rust
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
    let params = match parse_optional_params::<ResourceTemplatesListParams>(request.params) {
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
```

- [ ] **Step 4: Test template list**

Add this test in `main/src/server/mcp/service.rs`:

```rust
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
```

Run: `cargo test -p previa-main mcp::service::tests::resource_templates_include_dynamic_project_paths`

Expected: one test passes.

### Task 3: Implement Completion For Resource Arguments

- [ ] **Step 1: Add completion handler**

In `main/src/server/mcp/service.rs`, add this function:

```rust
async fn complete_mcp_argument(
    state: &AppState,
    params: CompletionCompleteParams,
) -> Result<CompletionResult, String> {
    let _ = params.meta.as_ref();
    let values = match params.reference {
        CompletionReference::Resource { uri } => {
            complete_resource_argument(state, &uri, &params.argument.name, &params.argument.value, &params.context).await?
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
```

- [ ] **Step 2: Add resource completion logic**

Add this helper below `complete_mcp_argument`:

```rust
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
            let project_id = completion_context_string(context, "projectId")
                .ok_or_else(|| "projectId context is required to complete pipelineRef".to_owned())?;
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
            Ok(filter_completion_values(specs.into_iter().map(|spec| spec.id), partial))
        }
        "testId" if uri_template.contains("/history/e2e/") => {
            complete_history_test_ids(state, uri_template, partial, true, context).await
        }
        "testId" if uri_template.contains("/history/load/") => {
            complete_history_test_ids(state, uri_template, partial, false, context).await
        }
        _ => Ok(Vec::new()),
    }
}
```

- [ ] **Step 3: Add shared filtering helpers**

Add:

```rust
fn completion_context_string(context: &CompletionContext, name: &str) -> Option<String> {
    context
        .arguments
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn filter_completion_values(values: impl IntoIterator<Item = String>, partial: &str) -> Vec<String> {
    values
        .into_iter()
        .filter(|value| value.starts_with(partial))
        .take(100)
        .collect()
}
```

- [ ] **Step 4: Add history completion helper**

Add:

```rust
async fn complete_history_test_ids(
    state: &AppState,
    _uri_template: &str,
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
        Ok(filter_completion_values(records.into_iter().map(|record| record.id), partial))
    } else {
        let records = list_load_history_records(&state.db, &project_id, query)
            .await
            .map_err(|err| format!("failed to complete load tests: {err}"))?;
        Ok(filter_completion_values(records.into_iter().map(|record| record.id), partial))
    }
}
```

- [ ] **Step 5: Handle `completion/complete`**

In `process_request`, add a match arm:

```rust
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
            response: McpResponse::success(request_id, serde_json::to_value(result).unwrap()),
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
```

- [ ] **Step 6: Add tests for filtering**

Add:

```rust
#[test]
fn completion_filter_matches_prefix_and_limits_results() {
    let values = (0..120).map(|index| format!("project-{index:03}"));

    let filtered = filter_completion_values(values, "project-0");

    assert_eq!(filtered.len(), 100);
    assert_eq!(filtered[0], "project-000");
    assert_eq!(filtered[99], "project-099");
}
```

Run: `cargo test -p previa-main mcp::service::tests::completion_filter_matches_prefix_and_limits_results`

Expected: one test passes.

---

## Plan 2: Toolsets And Output Schemas

**Goal:** Reduce tool noise and make Previa MCP calls easier for typed clients by grouping tools and declaring output schemas.

**Architecture:** Keep all tools implemented by the existing `execute_tool` match, but attach metadata to definitions and filter returned tools at `tools/list`. Add optional `outputSchema` to `ToolDefinition` without changing tool runtime behavior.

**Files:**
- Modify: `main/src/server/mcp/models.rs`
- Modify: `main/src/server/mcp/service.rs`
- Test: `main/src/server/mcp/service.rs`

### Task 1: Add Tool Metadata And Output Schema

- [ ] **Step 1: Extend `ToolDefinition`**

In `main/src/server/mcp/models.rs`, change `ToolDefinition` to:

```rust
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDefinition {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub description: String,
    pub input_schema: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "_meta")]
    pub meta: Option<Value>,
}
```

- [ ] **Step 2: Add a constructor helper**

In `main/src/server/mcp/service.rs`, add:

```rust
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
```

- [ ] **Step 3: Update existing definitions incrementally**

Start with these high-value tools in `tool_definitions()`:

```rust
tool_definition(
    "get_info",
    "Get Info",
    "Returns orchestrator health, runner registration, and runtime details.",
    json!({ "type": "object", "properties": {} }),
    Some(json!({
        "type": "object",
        "properties": {
            "runnerCount": { "type": "integer" },
            "runners": { "type": "array", "items": { "type": "object" } }
        }
    })),
    "core",
)
```

Then migrate one toolset at a time: `core`, `projects`, `pipelines`, `specs`, `e2e`, `load`, `admin`.

- [ ] **Step 4: Run tests to reveal compile misses**

Run: `cargo test -p previa-main mcp::service::tests::project_tools_require_project_id`

Expected: compiler errors identify any old `ToolDefinition` literals missing `output_schema` or `meta`; update all literals until the test passes.

### Task 2: Filter Tools By Toolset

- [ ] **Step 1: Add toolset parsing**

In `main/src/server/mcp/models.rs`, update `ToolsListParams`:

```rust
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolsListParams {
    pub cursor: Option<String>,
    #[serde(default)]
    pub toolsets: Vec<String>,
    #[serde(default, rename = "_meta")]
    pub meta: Option<Value>,
}
```

- [ ] **Step 2: Add filtering helper**

In `main/src/server/mcp/service.rs`, add:

```rust
fn filter_tools_by_toolset(tools: Vec<ToolDefinition>, requested: &[String]) -> Vec<ToolDefinition> {
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
```

- [ ] **Step 3: Apply filtering in `tools/list`**

Replace the current success branch with:

```rust
let tools = filter_tools_by_toolset(tool_definitions(), &params.toolsets);
McpHttpOutcome::Response {
    response: McpResponse::success(request_id, json!({ "tools": tools })),
    session_id: session_id.map(str::to_owned),
    protocol_version: Some(session.protocol_version),
}
```

- [ ] **Step 4: Add test**

Add:

```rust
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
```

Run: `cargo test -p previa-main mcp::service::tests::toolset_filter_returns_only_requested_group`

Expected: one test passes.

### Task 3: Add Output Schemas For Execution Tools

- [ ] **Step 1: Add reusable output schema helpers**

In `main/src/server/mcp/service.rs`, add:

```rust
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
```

- [ ] **Step 2: Attach schemas**

Use `Some(execution_start_output_schema())` for:
- `run_project_e2e_test`
- `run_project_load_test`
- `create_project_e2e_queue`

Use `Some(history_list_output_schema())` for:
- `list_e2e_history`
- `list_load_history`

- [ ] **Step 3: Add test**

Add:

```rust
#[test]
fn execution_tools_have_output_schemas() {
    let tools = tool_definitions();
    for name in ["run_project_e2e_test", "run_project_load_test"] {
        let tool = tools
            .iter()
            .find(|tool| tool.name == name)
            .expect("execution tool");
        assert!(tool.output_schema.is_some(), "{name} should expose outputSchema");
    }
}
```

Run: `cargo test -p previa-main mcp::service::tests::execution_tools_have_output_schemas`

Expected: one test passes.

---

## Plan 3: Operational Resources, Logging, And Safety Confirmations

**Goal:** Make the MCP server better for live operations by exposing current runtime state, emitting structured logs for long actions, and requiring explicit confirmation for destructive or high-risk calls.

**Architecture:** Add read-only resources for operational state first. Then add structured log notifications where the current request lifecycle already has enough context. Finally add a conservative confirmation token pattern that works with MCP clients even before full elicitation support is implemented.

**Files:**
- Modify: `main/src/server/mcp/models.rs`
- Modify: `main/src/server/mcp/service.rs`
- Modify: `main/src/server/execution/load.rs` only if execution log hooks need lower-level detail
- Test: `main/src/server/mcp/service.rs`

### Task 1: Add Operational Resources

- [ ] **Step 1: Add URI helpers**

In `main/src/server/mcp/service.rs`, add:

```rust
fn orchestrator_info_resource_uri() -> &'static str {
    "previa://orchestrator/info"
}

fn runners_resource_uri() -> &'static str {
    "previa://runners"
}

fn project_current_e2e_queue_resource_uri(project_id: &str) -> String {
    format!("{}/queues/e2e/current", project_resource_uri(project_id))
}
```

- [ ] **Step 2: List operational resources**

At the start of `list_resources`, include:

```rust
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
```

Inside the project loop, add:

```rust
resources.push(ResourceDefinition {
    uri: project_current_e2e_queue_resource_uri(&project.id),
    name: format!("project-{}-current-e2e-queue", project.id),
    title: Some(format!("{} Current E2E Queue", project.name)),
    description: Some("Current E2E queue snapshot when one is active.".to_owned()),
    mime_type: Some("application/json".to_owned()),
});
```

- [ ] **Step 3: Read operational resources**

In `read_resource`, add:

```rust
if uri == orchestrator_info_resource_uri() {
    let info = OrchestratorInfoResponse::from_state(state).await;
    return json_resource(uri, &info).map_err(|err| {
        ResourceReadError::Internal(format!("failed to encode resource: {err}"))
    });
}
```

If `OrchestratorInfoResponse::from_state` does not exist, use the same helper currently used by the `get_info` tool or extract the `get_info` body into `async fn build_orchestrator_info(state: &AppState) -> Result<OrchestratorInfoResponse, String>`.

Add a match arm:

```rust
["projects", project_id, "queues", "e2e", "current"] => {
    ensure_project_resource_exists(&state.db, project_id, uri).await?;
    let snapshot = get_current_e2e_queue_snapshot(state, project_id)
        .await
        .map_err(|err| ResourceReadError::Internal(format!("failed to load queue resource: {err}")))?;
    json_resource(uri, &snapshot).map_err(|err| {
        ResourceReadError::Internal(format!("failed to encode resource: {err}"))
    })
}
```

- [ ] **Step 4: Add test for URIs**

Add:

```rust
#[test]
fn operational_resource_uris_are_stable() {
    assert_eq!(orchestrator_info_resource_uri(), "previa://orchestrator/info");
    assert_eq!(runners_resource_uri(), "previa://runners");
    assert_eq!(
        project_current_e2e_queue_resource_uri("project-1"),
        "previa://projects/project-1/queues/e2e/current"
    );
}
```

Run: `cargo test -p previa-main mcp::service::tests::operational_resource_uris_are_stable`

Expected: one test passes.

### Task 2: Add Logging Capability And Structured Log Helper

- [ ] **Step 1: Advertise logging**

In `handle_initialize`, add:

```rust
"logging": {},
```

as a sibling of `tools`, `resources`, `prompts`, and `completions`.

- [ ] **Step 2: Add log level params**

In `main/src/server/mcp/models.rs`, add:

```rust
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoggingSetLevelParams {
    pub level: String,
    #[serde(default, rename = "_meta")]
    pub meta: Option<Value>,
}
```

- [ ] **Step 3: Accept `logging/setLevel`**

In `process_request`, add:

```rust
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
    if !["debug", "info", "notice", "warning", "error"].contains(&params.level.as_str()) {
        return McpHttpOutcome::Response {
            response: McpResponse::error(Some(request_id), INVALID_PARAMS, "unsupported log level"),
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
```

- [ ] **Step 4: Keep runtime logs in `tracing` first**

For the first implementation, do not stream server-originated notifications yet. Add structured `tracing::info!` calls around high-value MCP actions:

```rust
info!(
    tool_name = params.name,
    session_id = session_id.unwrap_or_default(),
    "mcp tool call started"
);
```

and after successful tool calls:

```rust
info!(
    tool_name = result_name,
    "mcp tool call completed"
);
```

Use a local `let result_name = params.name.clone();` before moving `params` into `execute_tool`.

- [ ] **Step 5: Add test for level validation helper**

Extract the level check into:

```rust
fn is_supported_mcp_log_level(level: &str) -> bool {
    matches!(level, "debug" | "info" | "notice" | "warning" | "error")
}
```

Add:

```rust
#[test]
fn mcp_log_level_validation_is_strict() {
    assert!(is_supported_mcp_log_level("info"));
    assert!(is_supported_mcp_log_level("warning"));
    assert!(!is_supported_mcp_log_level("verbose"));
}
```

Run: `cargo test -p previa-main mcp::service::tests::mcp_log_level_validation_is_strict`

Expected: one test passes.

### Task 3: Require Confirmation For High-Risk Tools

- [ ] **Step 1: Define risk classification**

In `main/src/server/mcp/service.rs`, add:

```rust
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
```

- [ ] **Step 2: Define confirmation token**

Add:

```rust
fn expected_confirmation_token(tool_name: &str, reason: &str) -> String {
    format!("confirm:{tool_name}:{reason}")
}

fn confirmation_token_from_arguments(arguments: &Value) -> Option<&str> {
    arguments
        .get("confirmationToken")
        .and_then(Value::as_str)
}
```

- [ ] **Step 3: Gate high-risk calls before execution**

In the `tools/call` match arm, after parsing `ToolCallParams` and before `execute_tool`, add:

```rust
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
}
```

- [ ] **Step 4: Add confirmation field to high-risk schemas**

For each high-risk tool input schema, add:

```rust
"confirmationToken": {
    "type": ["string", "null"],
    "description": "Required for high-risk calls after the server returns the exact confirmation token."
}
```

- [ ] **Step 5: Add tests**

Add:

```rust
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
```

Run: `cargo test -p previa-main mcp::service::tests::high_risk_load_test_requires_confirmation_at_high_intensity mcp::service::tests::normal_load_test_does_not_require_confirmation`

Expected: if Cargo rejects multiple filters, run `cargo test -p previa-main mcp::service::tests::high_risk`.

---

## Final Verification For Each Plan

- [ ] Run focused MCP tests for the implemented plan:

```bash
cargo test -p previa-main mcp::service::tests
```

Expected: all MCP service tests pass.

- [ ] Run formatting:

```bash
cargo fmt
```

Expected: no diff from formatter after the command.

- [ ] Run release build:

```bash
cargo build --release
```

Expected: release build completes successfully.

- [ ] Commit and push:

```bash
git status --short
git add main/src/server/mcp/models.rs main/src/server/mcp/service.rs docs/superpowers/plans/2026-05-11-mcp-capabilities-roadmap.md
git commit -m "feat: upgrade mcp capabilities"
git push
```

Expected: branch is pushed and working tree is clean.

---

## Self-Review

**Spec coverage:** The plan covers resource templates, completions, toolsets, output schemas, operational resources, logging, and safety confirmation. Sampling and full MCP elicitation are intentionally not in this first implementation because the existing server response type does not yet model multi round-trip requests.

**Placeholder scan:** No tasks rely on unfinished placeholder markers. Each code-changing task identifies exact files, snippets, tests, and expected commands.

**Type consistency:** New names are consistent across tasks: `ResourceTemplateDefinition`, `CompletionCompleteParams`, `ToolDefinition.output_schema`, `filter_tools_by_toolset`, and `high_risk_tool_reason`.
