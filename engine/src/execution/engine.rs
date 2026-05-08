use reqwest::Client;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use crate::assertions::{evaluate_assertions, has_status_assertion};
use crate::core::types::{
    Pipeline, RuntimeEnvGroup, RuntimeSpec, StepExecutionResult, StepRequest, StepResponse,
};
use crate::execution::cancel::await_with_cancel;
use crate::execution::http::{parse_absolute_http_url, parse_method};
use crate::execution::logging::{log_step_request, log_step_response};
use crate::template::resolve::resolve_template_variables;

fn noop_request_start_gate<'a>(
    _: &'a StepRequest,
) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>> {
    Box::pin(async { true })
}

pub async fn execute_pipeline(
    pipeline: &Pipeline,
    selected_base_url_key: Option<&str>,
) -> Vec<StepExecutionResult> {
    let client = Client::new();
    execute_pipeline_with_client_runtime_hooks(
        &client,
        pipeline,
        selected_base_url_key,
        None,
        None,
        None,
        |_| {},
        |_| {},
        || false,
        noop_request_start_gate,
    )
    .await
}

pub async fn execute_pipeline_with_client(
    client: &Client,
    pipeline: &Pipeline,
    selected_base_url_key: Option<&str>,
) -> Vec<StepExecutionResult> {
    execute_pipeline_with_client_runtime_hooks(
        client,
        pipeline,
        selected_base_url_key,
        None,
        None,
        None,
        |_| {},
        |_| {},
        || false,
        noop_request_start_gate,
    )
    .await
}

pub async fn execute_pipeline_with_hooks<FStart, FResult, FCancel>(
    pipeline: &Pipeline,
    selected_base_url_key: Option<&str>,
    on_step_start: FStart,
    on_step_result: FResult,
    should_cancel: FCancel,
) -> Vec<StepExecutionResult>
where
    FStart: FnMut(&str),
    FResult: FnMut(&StepExecutionResult),
    FCancel: FnMut() -> bool,
{
    let client = Client::new();
    execute_pipeline_with_client_runtime_hooks(
        &client,
        pipeline,
        selected_base_url_key,
        None,
        None,
        None,
        on_step_start,
        on_step_result,
        should_cancel,
        noop_request_start_gate,
    )
    .await
}

pub async fn execute_pipeline_with_specs_hooks<FStart, FResult, FCancel>(
    pipeline: &Pipeline,
    selected_base_url_key: Option<&str>,
    specs: Option<&[RuntimeSpec]>,
    on_step_start: FStart,
    on_step_result: FResult,
    should_cancel: FCancel,
) -> Vec<StepExecutionResult>
where
    FStart: FnMut(&str),
    FResult: FnMut(&StepExecutionResult),
    FCancel: FnMut() -> bool,
{
    let client = Client::new();
    execute_pipeline_with_client_runtime_hooks(
        &client,
        pipeline,
        selected_base_url_key,
        specs,
        None,
        None,
        on_step_start,
        on_step_result,
        should_cancel,
        noop_request_start_gate,
    )
    .await
}

pub async fn execute_pipeline_with_runtime_hooks<FStart, FResult, FCancel>(
    pipeline: &Pipeline,
    selected_base_url_key: Option<&str>,
    specs: Option<&[RuntimeSpec]>,
    env_groups: Option<&[RuntimeEnvGroup]>,
    selected_env_group_slug: Option<&str>,
    on_step_start: FStart,
    on_step_result: FResult,
    should_cancel: FCancel,
) -> Vec<StepExecutionResult>
where
    FStart: FnMut(&str),
    FResult: FnMut(&StepExecutionResult),
    FCancel: FnMut() -> bool,
{
    let client = Client::new();
    execute_pipeline_with_client_runtime_hooks(
        &client,
        pipeline,
        selected_base_url_key,
        specs,
        env_groups,
        selected_env_group_slug,
        on_step_start,
        on_step_result,
        should_cancel,
        noop_request_start_gate,
    )
    .await
}

pub async fn execute_pipeline_with_runtime_request_gate<FStart, FResult, FCancel, FGate>(
    pipeline: &Pipeline,
    selected_base_url_key: Option<&str>,
    specs: Option<&[RuntimeSpec]>,
    env_groups: Option<&[RuntimeEnvGroup]>,
    selected_env_group_slug: Option<&str>,
    on_step_start: FStart,
    on_step_result: FResult,
    should_cancel: FCancel,
    on_request_start: FGate,
) -> Vec<StepExecutionResult>
where
    FStart: FnMut(&str),
    FResult: FnMut(&StepExecutionResult),
    FCancel: FnMut() -> bool,
    FGate: for<'a> FnMut(&'a StepRequest) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>> + Send,
{
    let client = Client::new();
    execute_pipeline_with_client_runtime_hooks(
        &client,
        pipeline,
        selected_base_url_key,
        specs,
        env_groups,
        selected_env_group_slug,
        on_step_start,
        on_step_result,
        should_cancel,
        on_request_start,
    )
    .await
}

pub async fn execute_pipeline_with_client_runtime_request_gate<FStart, FResult, FCancel, FGate>(
    client: &Client,
    pipeline: &Pipeline,
    selected_base_url_key: Option<&str>,
    specs: Option<&[RuntimeSpec]>,
    env_groups: Option<&[RuntimeEnvGroup]>,
    selected_env_group_slug: Option<&str>,
    on_step_start: FStart,
    on_step_result: FResult,
    should_cancel: FCancel,
    on_request_start: FGate,
) -> Vec<StepExecutionResult>
where
    FStart: FnMut(&str),
    FResult: FnMut(&StepExecutionResult),
    FCancel: FnMut() -> bool,
    FGate: for<'a> FnMut(&'a StepRequest) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>> + Send,
{
    execute_pipeline_with_client_runtime_hooks(
        client,
        pipeline,
        selected_base_url_key,
        specs,
        env_groups,
        selected_env_group_slug,
        on_step_start,
        on_step_result,
        should_cancel,
        on_request_start,
    )
    .await
}

pub async fn execute_pipeline_with_client_hooks<FStart, FResult, FCancel>(
    client: &Client,
    pipeline: &Pipeline,
    selected_base_url_key: Option<&str>,
    on_step_start: FStart,
    on_step_result: FResult,
    should_cancel: FCancel,
) -> Vec<StepExecutionResult>
where
    FStart: FnMut(&str),
    FResult: FnMut(&StepExecutionResult),
    FCancel: FnMut() -> bool,
{
    execute_pipeline_with_client_runtime_hooks(
        client,
        pipeline,
        selected_base_url_key,
        None,
        None,
        None,
        on_step_start,
        on_step_result,
        should_cancel,
        noop_request_start_gate,
    )
    .await
}

fn finalize_step_result<FResult>(
    step_id: &str,
    result: StepExecutionResult,
    context: &mut HashMap<String, StepExecutionResult>,
    results: &mut Vec<StepExecutionResult>,
    on_step_result: &mut FResult,
) -> bool
where
    FResult: FnMut(&StepExecutionResult),
{
    let should_stop_pipeline = result.status == "error";
    context.insert(step_id.to_owned(), result.clone());
    on_step_result(&result);
    results.push(result);
    should_stop_pipeline
}

async fn execute_pipeline_with_client_runtime_hooks<FStart, FResult, FCancel, FGate>(
    client: &Client,
    pipeline: &Pipeline,
    selected_base_url_key: Option<&str>,
    specs: Option<&[RuntimeSpec]>,
    env_groups: Option<&[RuntimeEnvGroup]>,
    selected_env_group_slug: Option<&str>,
    on_step_start: FStart,
    on_step_result: FResult,
    should_cancel: FCancel,
    on_request_start: FGate,
) -> Vec<StepExecutionResult>
where
    FStart: FnMut(&str),
    FResult: FnMut(&StepExecutionResult),
    FCancel: FnMut() -> bool,
    FGate: for<'a> FnMut(&'a StepRequest) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>> + Send,
{
    execute_pipeline_with_client_runtime_hooks_from_index(
        client,
        pipeline,
        selected_base_url_key,
        0,
        HashMap::new(),
        specs,
        env_groups,
        selected_env_group_slug,
        on_step_start,
        on_step_result,
        should_cancel,
        on_request_start,
    )
    .await
}

pub async fn execute_pipeline_from_step_with_client_runtime_hooks<FStart, FResult, FCancel, FGate>(
    client: &Client,
    pipeline: &Pipeline,
    start_step_id: &str,
    initial_context: HashMap<String, StepExecutionResult>,
    specs: Option<&[RuntimeSpec]>,
    env_groups: Option<&[RuntimeEnvGroup]>,
    selected_env_group_slug: Option<&str>,
    on_step_start: FStart,
    on_step_result: FResult,
    should_cancel: FCancel,
    on_request_start: FGate,
) -> Vec<StepExecutionResult>
where
    FStart: FnMut(&str),
    FResult: FnMut(&StepExecutionResult),
    FCancel: FnMut() -> bool,
    FGate: for<'a> FnMut(&'a StepRequest) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>> + Send,
{
    let start_index = pipeline
        .steps
        .iter()
        .position(|step| step.id == start_step_id)
        .unwrap_or(pipeline.steps.len());

    execute_pipeline_with_client_runtime_hooks_from_index(
        client,
        pipeline,
        None,
        start_index,
        initial_context,
        specs,
        env_groups,
        selected_env_group_slug,
        on_step_start,
        on_step_result,
        should_cancel,
        on_request_start,
    )
    .await
}

async fn execute_pipeline_with_client_runtime_hooks_from_index<FStart, FResult, FCancel, FGate>(
    client: &Client,
    pipeline: &Pipeline,
    _selected_base_url_key: Option<&str>,
    start_index: usize,
    initial_context: HashMap<String, StepExecutionResult>,
    specs: Option<&[RuntimeSpec]>,
    env_groups: Option<&[RuntimeEnvGroup]>,
    selected_env_group_slug: Option<&str>,
    mut on_step_start: FStart,
    mut on_step_result: FResult,
    mut should_cancel: FCancel,
    mut on_request_start: FGate,
) -> Vec<StepExecutionResult>
where
    FStart: FnMut(&str),
    FResult: FnMut(&StepExecutionResult),
    FCancel: FnMut() -> bool,
    FGate: for<'a> FnMut(&'a StepRequest) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>> + Send,
{
    let mut context = initial_context;
    let mut results = Vec::with_capacity(pipeline.steps.len().saturating_sub(start_index));

    'steps: for step in pipeline.steps.iter().skip(start_index) {
        if should_cancel() {
            break;
        }

        let delay_ms = step.delay.unwrap_or(0);
        let max_attempts = step.retry.unwrap_or(0).saturating_add(1);

        for attempt in 1..=max_attempts {
            if should_cancel() {
                break 'steps;
            }

            if delay_ms > 0 {
                let Some(_) = await_with_cancel(
                    tokio::time::sleep(Duration::from_millis(delay_ms)),
                    &mut should_cancel,
                )
                .await
                else {
                    break 'steps;
                };
            }

            on_step_start(&step.id);
            let start = Instant::now();

            let resolved_url = resolve_template_variables(
                &Value::String(step.url.clone()),
                &context,
                specs,
                env_groups,
                selected_env_group_slug,
            )
            .as_str()
            .unwrap_or(step.url.as_str())
            .to_owned();

            let resolved_headers = resolve_template_variables(
                &serde_json::to_value(&step.headers).unwrap_or(Value::Object(Map::new())),
                &context,
                specs,
                env_groups,
                selected_env_group_slug,
            )
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned()))
                    .collect::<HashMap<String, String>>()
            })
            .unwrap_or_default();

            let resolved_body = step.body.as_ref().map(|body| {
                resolve_template_variables(
                    body,
                    &context,
                    specs,
                    env_groups,
                    selected_env_group_slug,
                )
            });

            let method = match parse_method(&step.method) {
                Ok(method) => method,
                Err(err) => {
                    let result = StepExecutionResult {
                        step_id: step.id.clone(),
                        status: "error".to_owned(),
                        request: Some(StepRequest {
                            method: step.method.clone(),
                            url: resolved_url.clone(),
                            headers: resolved_headers.clone(),
                            body: resolved_body.clone(),
                        }),
                        response: None,
                        error: Some(err),
                        duration: Some(start.elapsed().as_millis()),
                        attempts: if max_attempts > 1 {
                            Some(attempt)
                        } else {
                            None
                        },
                        attempt: Some(attempt),
                        max_attempts: Some(max_attempts),
                        assert_results: None,
                    };
                    log_step_response(&step.id, None, result.error.as_deref());

                    if attempt < max_attempts {
                        continue;
                    }
                    if finalize_step_result(
                        &step.id,
                        result,
                        &mut context,
                        &mut results,
                        &mut on_step_result,
                    ) {
                        break 'steps;
                    }
                    break;
                }
            };

            let url = match parse_absolute_http_url(&resolved_url) {
                Ok(url) => url,
                Err(err) => {
                    let result = StepExecutionResult {
                        step_id: step.id.clone(),
                        status: "error".to_owned(),
                        request: Some(StepRequest {
                            method: step.method.clone(),
                            url: resolved_url.clone(),
                            headers: resolved_headers.clone(),
                            body: resolved_body.clone(),
                        }),
                        response: None,
                        error: Some(err),
                        duration: Some(start.elapsed().as_millis()),
                        attempts: if max_attempts > 1 {
                            Some(attempt)
                        } else {
                            None
                        },
                        attempt: Some(attempt),
                        max_attempts: Some(max_attempts),
                        assert_results: None,
                    };
                    log_step_response(&step.id, None, result.error.as_deref());

                    if attempt < max_attempts {
                        continue;
                    }
                    if finalize_step_result(
                        &step.id,
                        result,
                        &mut context,
                        &mut results,
                        &mut on_step_result,
                    ) {
                        break 'steps;
                    }
                    break;
                }
            };

            let mut request_builder = client.request(method, url);

            for (k, v) in &resolved_headers {
                request_builder = request_builder.header(k, v);
            }

            if let Some(body) = &resolved_body {
                if !step.method.eq_ignore_ascii_case("GET")
                    && !step.method.eq_ignore_ascii_case("HEAD")
                {
                    request_builder = request_builder.json(body);
                }
            }

            let request = StepRequest {
                method: step.method.clone(),
                url: resolved_url.clone(),
                headers: resolved_headers.clone(),
                body: resolved_body.clone(),
            };
            log_step_request(&step.id, &request);
            let request_admitted = on_request_start(&request).await;
            if !request_admitted {
                break 'steps;
            }
            if should_cancel() {
                break 'steps;
            }

            let Some(send_result) =
                await_with_cancel(request_builder.send(), &mut should_cancel).await
            else {
                break 'steps;
            };

            match send_result {
                Ok(response) => {
                    let status = response.status();
                    let status_text = status.canonical_reason().unwrap_or("").to_owned();
                    let mut headers = HashMap::new();
                    for (k, v) in response.headers() {
                        headers.insert(
                            k.as_str().to_owned(),
                            v.to_str().unwrap_or_default().to_owned(),
                        );
                    }

                    let content_type = headers
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                        .map(|(_, v)| v.as_str())
                        .unwrap_or("");

                    let body = if content_type.contains("application/json") {
                        let Some(body_result) =
                            await_with_cancel(response.json::<Value>(), &mut should_cancel).await
                        else {
                            break 'steps;
                        };
                        match body_result {
                            Ok(value) => value,
                            Err(err) => {
                                let result = StepExecutionResult {
                                    step_id: step.id.clone(),
                                    status: "error".to_owned(),
                                    request: Some(request),
                                    response: None,
                                    error: Some(err.to_string()),
                                    duration: Some(start.elapsed().as_millis()),
                                    attempts: if max_attempts > 1 {
                                        Some(attempt)
                                    } else {
                                        None
                                    },
                                    attempt: Some(attempt),
                                    max_attempts: Some(max_attempts),
                                    assert_results: None,
                                };
                                log_step_response(&step.id, None, result.error.as_deref());

                                if attempt < max_attempts {
                                    continue;
                                }
                                if finalize_step_result(
                                    &step.id,
                                    result,
                                    &mut context,
                                    &mut results,
                                    &mut on_step_result,
                                ) {
                                    break 'steps;
                                }
                                break;
                            }
                        }
                    } else {
                        let Some(body_result) =
                            await_with_cancel(response.text(), &mut should_cancel).await
                        else {
                            break 'steps;
                        };
                        Value::String(body_result.unwrap_or_default())
                    };

                    let http_error = (!status.is_success())
                        .then(|| format!("HTTP {} {}", status.as_u16(), status_text));
                    let mut result = StepExecutionResult {
                        step_id: step.id.clone(),
                        status: "success".to_owned(),
                        request: Some(request),
                        response: Some(StepResponse {
                            status: status.as_u16(),
                            status_text: status_text.clone(),
                            headers,
                            body,
                        }),
                        error: http_error.clone(),
                        duration: Some(start.elapsed().as_millis()),
                        attempts: if max_attempts > 1 {
                            Some(attempt)
                        } else {
                            None
                        },
                        attempt: Some(attempt),
                        max_attempts: Some(max_attempts),
                        assert_results: None,
                    };

                    let has_status_assert = has_status_assertion(step);
                    let assert_results = evaluate_assertions(
                        step,
                        &result,
                        &context,
                        specs,
                        env_groups,
                        selected_env_group_slug,
                    );
                    let assertion_failed = assert_results.iter().any(|r| !r.passed);
                    if !assert_results.is_empty() {
                        if assertion_failed {
                            result.status = "error".to_owned();
                            let failed_count = assert_results.iter().filter(|r| !r.passed).count();
                            result.error = Some(match result.error {
                                Some(err) => {
                                    format!("{} | {} assertion(s) failed", err, failed_count)
                                }
                                None => format!("{} assertion(s) failed", failed_count),
                            });
                        } else if http_error.is_some() {
                            if has_status_assert {
                                result.status = "success".to_owned();
                                result.error = None;
                            } else {
                                result.status = "error".to_owned();
                            }
                        }
                        result.assert_results = Some(assert_results);
                    } else if http_error.is_some() {
                        result.status = "error".to_owned();
                    }

                    log_step_response(&step.id, result.response.as_ref(), result.error.as_deref());

                    if assertion_failed && attempt < max_attempts {
                        continue;
                    }

                    if finalize_step_result(
                        &step.id,
                        result,
                        &mut context,
                        &mut results,
                        &mut on_step_result,
                    ) {
                        break 'steps;
                    }
                    break;
                }
                Err(err) => {
                    let result = StepExecutionResult {
                        step_id: step.id.clone(),
                        status: "error".to_owned(),
                        request: Some(request),
                        response: None,
                        error: Some(err.to_string()),
                        duration: Some(start.elapsed().as_millis()),
                        attempts: if max_attempts > 1 {
                            Some(attempt)
                        } else {
                            None
                        },
                        attempt: Some(attempt),
                        max_attempts: Some(max_attempts),
                        assert_results: None,
                    };
                    log_step_response(&step.id, None, result.error.as_deref());

                    if attempt < max_attempts {
                        continue;
                    }

                    if finalize_step_result(
                        &step.id,
                        result,
                        &mut context,
                        &mut results,
                        &mut on_step_result,
                    ) {
                        break 'steps;
                    }
                    break;
                }
            }
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::{
        Pipeline, PipelineStep, RuntimeSpec, StepAssertion, StepRequest, StepResponse,
    };
    use httpmock::Method::{GET, POST};
    use httpmock::MockServer;
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test]
    async fn executes_from_step_with_seeded_previous_results() {
        let server = MockServer::start_async().await;
        let protected = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/protected")
                    .header("authorization", "Bearer abc123");
                then.status(200)
                    .header("content-type", "application/json")
                    .json_body(json!({ "ok": true }));
            })
            .await;

        let pipeline = Pipeline {
            id: Some("pipe-1".to_owned()),
            name: "Pipe".to_owned(),
            description: None,
            steps: vec![
                PipelineStep {
                    id: "login".to_owned(),
                    name: "Login".to_owned(),
                    description: None,
                    method: "POST".to_owned(),
                    url: format!("{}/login", server.base_url()),
                    headers: HashMap::new(),
                    body: None,
                    operation_id: None,
                    delay: None,
                    retry: None,
                    asserts: Vec::new(),
                },
                PipelineStep {
                    id: "protected".to_owned(),
                    name: "Protected".to_owned(),
                    description: None,
                    method: "GET".to_owned(),
                    url: format!("{}/protected", server.base_url()),
                    headers: HashMap::from([(
                        "Authorization".to_owned(),
                        "Bearer {{steps.login.token}}".to_owned(),
                    )]),
                    body: None,
                    operation_id: None,
                    delay: None,
                    retry: None,
                    asserts: Vec::new(),
                },
            ],
        };

        let seeded = HashMap::from([(
            "login".to_owned(),
            StepExecutionResult {
                step_id: "login".to_owned(),
                status: "success".to_owned(),
                request: Some(StepRequest {
                    method: "POST".to_owned(),
                    url: format!("{}/login", server.base_url()),
                    headers: HashMap::new(),
                    body: None,
                }),
                response: Some(StepResponse {
                    status: 200,
                    status_text: "OK".to_owned(),
                    headers: HashMap::new(),
                    body: json!({ "token": "abc123" }),
                }),
                error: None,
                duration: Some(1),
                attempts: Some(1),
                attempt: Some(1),
                max_attempts: Some(1),
                assert_results: None,
            },
        )]);

        let mut started = Vec::new();
        let results = execute_pipeline_from_step_with_client_runtime_hooks(
            &reqwest::Client::new(),
            &pipeline,
            "protected",
            seeded,
            None,
            None,
            None,
            |step_id| started.push(step_id.to_owned()),
            |_| {},
            || false,
            |_| Box::pin(async { true }),
        )
        .await;

        assert_eq!(started, vec!["protected"]);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].step_id, "protected");
        assert_eq!(results[0].status, "success");
        protected.assert_async().await;
    }

    #[tokio::test]
    async fn executes_pipeline_with_interpolation_and_assertions() {
        let server = MockServer::start_async().await;

        let create_user = server
            .mock_async(|when, then| {
                when.method(POST).path("/users");
                then.status(201)
                    .header("content-type", "application/json")
                    .json_body(
                        json!({ "id": "u-1", "token": "token-123", "email": "john@example.com" }),
                    );
            })
            .await;

        let get_user = server
            .mock_async(|when, then| {
                when.method(GET).path("/users/u-1");
                then.status(200)
                    .header("content-type", "application/json")
                    .json_body(json!({ "id": "u-1", "email": "john@example.com" }));
            })
            .await;

        let pipeline = Pipeline {
            id: None,
            name: "User flow".to_owned(),
            description: Some("Pipeline test".to_owned()),
            steps: vec![
                PipelineStep {
                    id: "create_user".to_owned(),
                    name: "Create".to_owned(),
                    description: None,
                    method: "POST".to_owned(),
                    url: format!("{}/users", server.base_url()),
                    headers: HashMap::from([(
                        "content-type".to_owned(),
                        "application/json".to_owned(),
                    )]),
                    body: Some(json!({ "name": "{{helpers.name}}" })),
                    operation_id: None,
                    delay: None,
                    retry: None,
                    asserts: vec![
                        StepAssertion {
                            field: "status".to_owned(),
                            operator: "equals".to_owned(),
                            expected: Some("201".to_owned()),
                        },
                        StepAssertion {
                            field: "body.id".to_owned(),
                            operator: "exists".to_owned(),
                            expected: None,
                        },
                    ],
                },
                PipelineStep {
                    id: "get_user".to_owned(),
                    name: "Get".to_owned(),
                    description: None,
                    method: "GET".to_owned(),
                    url: format!("{}/users/{{{{steps.create_user.id}}}}", server.base_url()),
                    headers: HashMap::new(),
                    body: None,
                    operation_id: None,
                    delay: None,
                    retry: None,
                    asserts: vec![
                        StepAssertion {
                            field: "status".to_owned(),
                            operator: "equals".to_owned(),
                            expected: Some("{{steps.create_user.status}}".to_owned()),
                        },
                        StepAssertion {
                            field: "body.email".to_owned(),
                            operator: "contains".to_owned(),
                            expected: Some("@".to_owned()),
                        },
                    ],
                },
            ],
        };

        let results = execute_pipeline(&pipeline, None).await;

        create_user.assert_async().await;
        get_user.assert_async().await;

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].status, "success");
        assert_eq!(results[1].status, "error");
        assert!(
            results[1]
                .error
                .as_ref()
                .is_some_and(|err| err.contains("assertion(s) failed"))
        );
    }

    #[tokio::test]
    async fn resolves_spec_url_variable() {
        let server_dev = MockServer::start_async().await;
        let server_prod = MockServer::start_async().await;

        let _dev = server_dev
            .mock_async(|when, then| {
                when.method(GET).path("/health");
                then.status(200).body("dev");
            })
            .await;

        let prod = server_prod
            .mock_async(|when, then| {
                when.method(GET).path("/health");
                then.status(200).body("prod");
            })
            .await;

        let pipeline = Pipeline {
            id: None,
            name: "Env".to_owned(),
            description: None,
            steps: vec![PipelineStep {
                id: "health".to_owned(),
                name: "Health".to_owned(),
                description: None,
                method: "GET".to_owned(),
                url: "{{specs.users-api.url.prod}}/health".to_owned(),
                headers: HashMap::new(),
                body: None,
                operation_id: None,
                delay: None,
                retry: None,
                asserts: vec![],
            }],
        };
        let specs = [RuntimeSpec {
            slug: "users-api".to_owned(),
            servers: HashMap::from([
                ("dev".to_owned(), server_dev.base_url()),
                ("prod".to_owned(), server_prod.base_url()),
            ]),
        }];

        let results = execute_pipeline_with_specs_hooks(
            &pipeline,
            Some("dev"),
            Some(&specs),
            |_| {},
            |_| {},
            || false,
        )
        .await;

        prod.assert_async().await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, "success");
        assert_eq!(
            results[0].response.as_ref().map(|r| r.body.clone()),
            Some(Value::String("prod".to_owned()))
        );
    }

    #[tokio::test]
    async fn request_gate_can_decline_before_http_send() {
        let server = MockServer::start_async().await;
        let call = server
            .mock_async(|when, then| {
                when.method(GET).path("/blocked");
                then.status(200).body("should not be called");
            })
            .await;

        let pipeline = Pipeline {
            id: None,
            name: "Gate".to_owned(),
            description: None,
            steps: vec![PipelineStep {
                id: "blocked".to_owned(),
                name: "Blocked".to_owned(),
                description: None,
                method: "GET".to_owned(),
                url: format!("{}/blocked", server.base_url()),
                headers: HashMap::new(),
                body: None,
                operation_id: None,
                delay: None,
                retry: None,
                asserts: vec![],
            }],
        };

        let results = execute_pipeline_with_runtime_request_gate(
            &pipeline,
            None,
            None,
            None,
            None,
            |_| {},
            |_| {},
            || false,
            |_| Box::pin(async { false }),
        )
        .await;

        assert!(results.is_empty());
        call.assert_calls_async(0).await;
    }

    #[tokio::test]
    async fn client_runtime_request_gate_uses_provided_client() {
        let server = MockServer::start_async().await;
        let call = server
            .mock_async(|when, then| {
                when.method(GET).path("/shared-client");
                then.status(200).body("ok");
            })
            .await;

        let pipeline = Pipeline {
            id: None,
            name: "Shared client".to_owned(),
            description: None,
            steps: vec![PipelineStep {
                id: "shared".to_owned(),
                name: "Shared".to_owned(),
                description: None,
                method: "GET".to_owned(),
                url: format!("{}/shared-client", server.base_url()),
                headers: HashMap::new(),
                body: None,
                operation_id: None,
                delay: None,
                retry: None,
                asserts: vec![],
            }],
        };

        let client = Client::new();
        let results = execute_pipeline_with_client_runtime_request_gate(
            &client,
            &pipeline,
            None,
            None,
            None,
            None,
            |_| {},
            |_| {},
            || false,
            |_| Box::pin(async { true }),
        )
        .await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, "success");
        call.assert_calls_async(1).await;
    }

    #[tokio::test]
    async fn marks_step_as_error_when_assertion_fails() {
        let server = MockServer::start_async().await;

        let call = server
            .mock_async(|when, then| {
                when.method(GET).path("/status");
                then.status(200)
                    .header("content-type", "application/json")
                    .json_body(json!({ "ok": true }));
            })
            .await;

        let pipeline = Pipeline {
            id: None,
            name: "Assert".to_owned(),
            description: None,
            steps: vec![PipelineStep {
                id: "status".to_owned(),
                name: "Status".to_owned(),
                description: None,
                method: "GET".to_owned(),
                url: format!("{}/status", server.base_url()),
                headers: HashMap::new(),
                body: None,
                operation_id: None,
                delay: None,
                retry: None,
                asserts: vec![StepAssertion {
                    field: "status".to_owned(),
                    operator: "equals".to_owned(),
                    expected: Some("201".to_owned()),
                }],
            }],
        };

        let results = execute_pipeline(&pipeline, None).await;

        call.assert_async().await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, "error");
        assert!(
            results[0]
                .error
                .as_ref()
                .is_some_and(|err| err.contains("1 assertion(s) failed"))
        );
    }

    #[tokio::test]
    async fn stops_pipeline_after_step_failure() {
        let server = MockServer::start_async().await;

        let failing_step = server
            .mock_async(|when, then| {
                when.method(GET).path("/fails");
                then.status(500).body("internal error");
            })
            .await;

        let next_step = server
            .mock_async(|when, then| {
                when.method(GET).path("/next");
                then.status(200).body("ok");
            })
            .await;

        let pipeline = Pipeline {
            id: None,
            name: "Stop on failure".to_owned(),
            description: None,
            steps: vec![
                PipelineStep {
                    id: "fails".to_owned(),
                    name: "Fails".to_owned(),
                    description: None,
                    method: "GET".to_owned(),
                    url: format!("{}/fails", server.base_url()),
                    headers: HashMap::new(),
                    body: None,
                    operation_id: None,
                    delay: None,
                    retry: None,
                    asserts: vec![StepAssertion {
                        field: "status".to_owned(),
                        operator: "equals".to_owned(),
                        expected: Some("201".to_owned()),
                    }],
                },
                PipelineStep {
                    id: "next".to_owned(),
                    name: "Next".to_owned(),
                    description: None,
                    method: "GET".to_owned(),
                    url: format!("{}/next", server.base_url()),
                    headers: HashMap::new(),
                    body: None,
                    operation_id: None,
                    delay: None,
                    retry: None,
                    asserts: vec![],
                },
            ],
        };

        let results = execute_pipeline(&pipeline, None).await;

        failing_step.assert_async().await;
        assert_eq!(next_step.calls_async().await, 0);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].step_id, "fails");
        assert_eq!(results[0].status, "error");
        assert!(
            results[0]
                .error
                .as_ref()
                .is_some_and(|err| err.contains("HTTP 500") && err.contains("assertion(s) failed"))
        );
    }

    #[tokio::test]
    async fn executes_create_user_and_send_email_case_from_json_payload() {
        let server = MockServer::start_async().await;

        let create_user = server
            .mock_async(|when, then| {
                when.method(POST).path("/users");
                then.status(201)
                    .header("content-type", "application/json")
                    .json_body(json!({
                        "$id": "usr-100",
                        "name": "John Doe",
                        "email": "john@example.com"
                    }));
            })
            .await;

        let send_email = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/emails")
                    .json_body(json!({ "to": "john@example.com", "name": "John Doe" }));
                then.status(201)
                    .header("content-type", "application/json")
                    .json_body(json!({ "queued": true }));
            })
            .await;

        let payload = json!({
            "name": "Criar Usuário e Enviar Email",
            "description": "Pipeline de cadastro completo",
            "steps": [
                {
                    "id": "create_user",
                    "name": "Criar Usuário",
                    "description": "Cria um novo usuário com dados aleatórios",
                    "headers": {
                        "Content-Type": "application/json"
                    },
                    "method": "POST",
                    "url": format!("{}/users", server.base_url()),
                    "body": {
                        "id": "{{helpers.uuid}}",
                        "name": "{{helpers.name}}",
                        "email": "{{helpers.email}}",
                        "cpf": "{{helpers.cpf}}"
                    },
                    "operationId": "createUser",
                    "asserts": [
                        {
                            "field": "status",
                            "operator": "equals",
                            "expected": "201"
                        },
                        {
                            "field": "body.$id",
                            "operator": "exists"
                        },
                        {
                            "field": "body.email",
                            "operator": "contains",
                            "expected": "@"
                        }
                    ]
                },
                {
                    "id": "send_email",
                    "name": "Enviar Email de Boas-Vindas",
                    "description": "Envia email usando dados do step anterior",
                    "headers": {
                        "Content-Type": "application/json"
                    },
                    "method": "POST",
                    "url": format!("{}/emails", server.base_url()),
                    "body": {
                        "to": "{{steps.create_user.email}}",
                        "name": "{{steps.create_user.name}}"
                    },
                    "asserts": [
                        {
                            "field": "status",
                            "operator": "equals",
                            "expected": "201"
                        }
                    ]
                }
            ],
            "id": "e3045988"
        });

        let pipeline: Pipeline =
            serde_json::from_value(payload).expect("pipeline payload is valid");
        let results = execute_pipeline(&pipeline, None).await;

        create_user.assert_async().await;
        send_email.assert_async().await;

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].status, "success");
        assert_eq!(results[1].status, "success");
        assert_eq!(pipeline.id, Some("e3045988".to_owned()));
        assert_eq!(
            pipeline.steps[0].operation_id.as_deref(),
            Some("createUser")
        );
        assert!(
            results[0]
                .request
                .as_ref()
                .and_then(|r| r.body.as_ref())
                .and_then(|b| b.get("cpf"))
                .and_then(|v| v.as_str())
                .is_some_and(|cpf| cpf.len() == 14 && cpf.contains('.') && cpf.contains('-'))
        );
    }

    #[tokio::test]
    async fn cancels_in_flight_future_when_cancel_flag_changes() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_writer = Arc::clone(&cancelled);

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            cancelled_writer.store(true, Ordering::SeqCst);
        });

        let mut should_cancel = || cancelled.load(Ordering::SeqCst);
        let result = await_with_cancel(
            async {
                tokio::time::sleep(Duration::from_millis(500)).await;
                "done"
            },
            &mut should_cancel,
        )
        .await;

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn retries_when_assertions_fail() {
        let server = MockServer::start_async().await;
        let status_call = server
            .mock_async(|when, then| {
                when.method(GET).path("/status");
                then.status(200)
                    .header("content-type", "application/json")
                    .json_body(json!({ "ok": false }));
            })
            .await;

        let pipeline = Pipeline {
            id: None,
            name: "Retry assertions".to_owned(),
            description: None,
            steps: vec![PipelineStep {
                id: "status".to_owned(),
                name: "Status".to_owned(),
                description: None,
                method: "GET".to_owned(),
                url: format!("{}/status", server.base_url()),
                headers: HashMap::new(),
                body: None,
                operation_id: None,
                delay: Some(0),
                retry: Some(2),
                asserts: vec![StepAssertion {
                    field: "body.ok".to_owned(),
                    operator: "equals".to_owned(),
                    expected: Some("true".to_owned()),
                }],
            }],
        };

        let results = execute_pipeline(&pipeline, None).await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, "error");
        assert_eq!(results[0].attempt, Some(3));
        assert_eq!(results[0].max_attempts, Some(3));
        status_call.assert_calls_async(3).await;
    }

    #[tokio::test]
    async fn does_not_retry_on_http_error_without_assertions() {
        let server = MockServer::start_async().await;
        let call = server
            .mock_async(|when, then| {
                when.method(GET).path("/fails");
                then.status(500).body("internal error");
            })
            .await;

        let pipeline = Pipeline {
            id: None,
            name: "No retry on HTTP".to_owned(),
            description: None,
            steps: vec![PipelineStep {
                id: "fails".to_owned(),
                name: "Fails".to_owned(),
                description: None,
                method: "GET".to_owned(),
                url: format!("{}/fails", server.base_url()),
                headers: HashMap::new(),
                body: None,
                operation_id: None,
                delay: Some(0),
                retry: Some(5),
                asserts: vec![],
            }],
        };

        let results = execute_pipeline(&pipeline, None).await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, "error");
        assert_eq!(results[0].attempt, Some(1));
        assert_eq!(results[0].max_attempts, Some(6));
        call.assert_calls_async(1).await;
    }

    #[tokio::test]
    async fn accepts_404_when_status_assert_matches() {
        let server = MockServer::start_async().await;
        let call = server
            .mock_async(|when, then| {
                when.method(GET).path("/missing");
                then.status(404)
                    .header("content-type", "application/json")
                    .json_body(json!({ "message": "not found" }));
            })
            .await;

        let pipeline = Pipeline {
            id: None,
            name: "Expected 404".to_owned(),
            description: None,
            steps: vec![PipelineStep {
                id: "missing".to_owned(),
                name: "Missing".to_owned(),
                description: None,
                method: "GET".to_owned(),
                url: format!("{}/missing", server.base_url()),
                headers: HashMap::new(),
                body: None,
                operation_id: None,
                delay: None,
                retry: Some(5),
                asserts: vec![StepAssertion {
                    field: "status".to_owned(),
                    operator: "equals".to_owned(),
                    expected: Some("404".to_owned()),
                }],
            }],
        };

        let results = execute_pipeline(&pipeline, None).await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, "success");
        assert_eq!(results[0].error, None);
        assert_eq!(results[0].attempt, Some(1));
        assert_eq!(
            results[0]
                .response
                .as_ref()
                .and_then(|response| response.body.get("message"))
                .and_then(|value| value.as_str()),
            Some("not found")
        );
        call.assert_calls_async(1).await;
    }

    #[tokio::test]
    async fn accepts_500_when_status_assert_matches() {
        let server = MockServer::start_async().await;
        let call = server
            .mock_async(|when, then| {
                when.method(GET).path("/boom");
                then.status(500).body("internal error");
            })
            .await;

        let pipeline = Pipeline {
            id: None,
            name: "Expected 500".to_owned(),
            description: None,
            steps: vec![PipelineStep {
                id: "boom".to_owned(),
                name: "Boom".to_owned(),
                description: None,
                method: "GET".to_owned(),
                url: format!("{}/boom", server.base_url()),
                headers: HashMap::new(),
                body: None,
                operation_id: None,
                delay: None,
                retry: None,
                asserts: vec![StepAssertion {
                    field: "status".to_owned(),
                    operator: "equals".to_owned(),
                    expected: Some("500".to_owned()),
                }],
            }],
        };

        let results = execute_pipeline(&pipeline, None).await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, "success");
        assert_eq!(results[0].error, None);
        assert_eq!(
            results[0]
                .response
                .as_ref()
                .map(|response| response.body.clone()),
            Some(Value::String("internal error".to_owned()))
        );
        call.assert_calls_async(1).await;
    }

    #[tokio::test]
    async fn accepts_array_index_assertions_in_response_body() {
        let server = MockServer::start_async().await;
        let call = server
            .mock_async(|when, then| {
                when.method(GET).path("/runtime");
                then.status(200)
                    .header("content-type", "application/json")
                    .json_body(json!({
                        "app": {
                            "status": "ready"
                        },
                        "pods": [
                            {
                                "podName": "app-keep-manual-123",
                                "phase": "Running"
                            }
                        ],
                        "containers": [
                            {
                                "name": "app-keep-manual"
                            }
                        ]
                    }));
            })
            .await;

        let pipeline = Pipeline {
            id: None,
            name: "Runtime arrays".to_owned(),
            description: None,
            steps: vec![PipelineStep {
                id: "runtime".to_owned(),
                name: "Runtime".to_owned(),
                description: None,
                method: "GET".to_owned(),
                url: format!("{}/runtime", server.base_url()),
                headers: HashMap::new(),
                body: None,
                operation_id: None,
                delay: None,
                retry: None,
                asserts: vec![
                    StepAssertion {
                        field: "status".to_owned(),
                        operator: "equals".to_owned(),
                        expected: Some("200".to_owned()),
                    },
                    StepAssertion {
                        field: "body.app.status".to_owned(),
                        operator: "equals".to_owned(),
                        expected: Some("ready".to_owned()),
                    },
                    StepAssertion {
                        field: "body.pods.0.podName".to_owned(),
                        operator: "exists".to_owned(),
                        expected: None,
                    },
                    StepAssertion {
                        field: "body.pods.0.phase".to_owned(),
                        operator: "equals".to_owned(),
                        expected: Some("Running".to_owned()),
                    },
                    StepAssertion {
                        field: "body.containers.0.name".to_owned(),
                        operator: "exists".to_owned(),
                        expected: None,
                    },
                ],
            }],
        };

        let results = execute_pipeline(&pipeline, None).await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, "success");
        assert_eq!(results[0].error, None);
        assert!(
            results[0]
                .assert_results
                .as_ref()
                .is_some_and(|items| items.iter().all(|item| item.passed))
        );
        call.assert_calls_async(1).await;
    }

    #[tokio::test]
    async fn retries_when_status_assert_fails_on_http_error() {
        let server = MockServer::start_async().await;
        let call = server
            .mock_async(|when, then| {
                when.method(GET).path("/missing");
                then.status(404).body("not found");
            })
            .await;

        let pipeline = Pipeline {
            id: None,
            name: "Retry status assert".to_owned(),
            description: None,
            steps: vec![PipelineStep {
                id: "missing".to_owned(),
                name: "Missing".to_owned(),
                description: None,
                method: "GET".to_owned(),
                url: format!("{}/missing", server.base_url()),
                headers: HashMap::new(),
                body: None,
                operation_id: None,
                delay: None,
                retry: Some(2),
                asserts: vec![StepAssertion {
                    field: "status".to_owned(),
                    operator: "equals".to_owned(),
                    expected: Some("200".to_owned()),
                }],
            }],
        };

        let results = execute_pipeline(&pipeline, None).await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, "error");
        assert_eq!(results[0].attempt, Some(3));
        assert_eq!(results[0].max_attempts, Some(3));
        assert!(
            results[0]
                .error
                .as_ref()
                .is_some_and(|err| err.contains("HTTP 404"))
        );
        call.assert_calls_async(3).await;
    }

    #[tokio::test]
    async fn keeps_http_error_when_status_assert_passes_but_body_assert_fails() {
        let server = MockServer::start_async().await;
        let call = server
            .mock_async(|when, then| {
                when.method(GET).path("/missing");
                then.status(404)
                    .header("content-type", "application/json")
                    .json_body(json!({ "message": "not found" }));
            })
            .await;

        let pipeline = Pipeline {
            id: None,
            name: "HTTP plus body assert failure".to_owned(),
            description: None,
            steps: vec![PipelineStep {
                id: "missing".to_owned(),
                name: "Missing".to_owned(),
                description: None,
                method: "GET".to_owned(),
                url: format!("{}/missing", server.base_url()),
                headers: HashMap::new(),
                body: None,
                operation_id: None,
                delay: None,
                retry: Some(1),
                asserts: vec![
                    StepAssertion {
                        field: "status".to_owned(),
                        operator: "equals".to_owned(),
                        expected: Some("404".to_owned()),
                    },
                    StepAssertion {
                        field: "body.code".to_owned(),
                        operator: "equals".to_owned(),
                        expected: Some("x".to_owned()),
                    },
                ],
            }],
        };

        let results = execute_pipeline(&pipeline, None).await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, "error");
        assert_eq!(results[0].attempt, Some(2));
        assert_eq!(results[0].max_attempts, Some(2));
        assert!(
            results[0]
                .error
                .as_ref()
                .is_some_and(|err| err.contains("HTTP 404") && err.contains("assertion(s) failed"))
        );
        call.assert_calls_async(2).await;
    }

    #[tokio::test]
    async fn keeps_http_error_without_status_assert_even_if_body_assert_passes() {
        let server = MockServer::start_async().await;
        let call = server
            .mock_async(|when, then| {
                when.method(GET).path("/missing");
                then.status(404)
                    .header("content-type", "application/json")
                    .json_body(json!({ "message": "not found" }));
            })
            .await;

        let pipeline = Pipeline {
            id: None,
            name: "Body assert only".to_owned(),
            description: None,
            steps: vec![PipelineStep {
                id: "missing".to_owned(),
                name: "Missing".to_owned(),
                description: None,
                method: "GET".to_owned(),
                url: format!("{}/missing", server.base_url()),
                headers: HashMap::new(),
                body: None,
                operation_id: None,
                delay: None,
                retry: Some(3),
                asserts: vec![StepAssertion {
                    field: "body.message".to_owned(),
                    operator: "equals".to_owned(),
                    expected: Some("not found".to_owned()),
                }],
            }],
        };

        let results = execute_pipeline(&pipeline, None).await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, "error");
        assert_eq!(results[0].attempt, Some(1));
        assert_eq!(results[0].max_attempts, Some(4));
        assert!(
            results[0]
                .error
                .as_ref()
                .is_some_and(|err| err.contains("HTTP 404"))
        );
        call.assert_calls_async(1).await;
    }

    #[tokio::test]
    async fn accepts_http_error_when_status_assert_uses_other_operator() {
        let server = MockServer::start_async().await;
        let call = server
            .mock_async(|when, then| {
                when.method(GET).path("/missing");
                then.status(404).body("not found");
            })
            .await;

        let pipeline = Pipeline {
            id: None,
            name: "Expected non-200".to_owned(),
            description: None,
            steps: vec![PipelineStep {
                id: "missing".to_owned(),
                name: "Missing".to_owned(),
                description: None,
                method: "GET".to_owned(),
                url: format!("{}/missing", server.base_url()),
                headers: HashMap::new(),
                body: None,
                operation_id: None,
                delay: None,
                retry: None,
                asserts: vec![StepAssertion {
                    field: "status".to_owned(),
                    operator: "not_equals".to_owned(),
                    expected: Some("200".to_owned()),
                }],
            }],
        };

        let results = execute_pipeline(&pipeline, None).await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, "success");
        assert_eq!(results[0].error, None);
        call.assert_calls_async(1).await;
    }

    #[tokio::test]
    async fn applies_delay_before_each_attempt() {
        let server = MockServer::start_async().await;
        let call = server
            .mock_async(|when, then| {
                when.method(GET).path("/delayed");
                then.status(200)
                    .header("content-type", "application/json")
                    .json_body(json!({ "ok": false }));
            })
            .await;

        let pipeline = Pipeline {
            id: None,
            name: "Delay before attempts".to_owned(),
            description: None,
            steps: vec![PipelineStep {
                id: "delayed".to_owned(),
                name: "Delayed".to_owned(),
                description: None,
                method: "GET".to_owned(),
                url: format!("{}/delayed", server.base_url()),
                headers: HashMap::new(),
                body: None,
                operation_id: None,
                delay: Some(30),
                retry: Some(2),
                asserts: vec![StepAssertion {
                    field: "body.ok".to_owned(),
                    operator: "equals".to_owned(),
                    expected: Some("true".to_owned()),
                }],
            }],
        };

        let started = std::time::Instant::now();
        let results = execute_pipeline(&pipeline, None).await;
        let elapsed = started.elapsed();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].attempt, Some(3));
        assert_eq!(results[0].max_attempts, Some(3));
        assert!(elapsed >= Duration::from_millis(75));
        call.assert_calls_async(3).await;
    }
}
