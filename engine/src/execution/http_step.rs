use reqwest::{Client, Method, Url};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::future::Future;
use std::time::Instant;

use crate::assertions::{evaluate_assertions, has_status_assertion};
use crate::core::types::{
    PipelineStep, RuntimeEnvGroup, RuntimeSpec, StepExecutionResult, StepRequest, StepResponse,
};
use crate::execution::cancel::await_with_cancel;
use crate::execution::http::{parse_absolute_http_url, parse_method};
use crate::execution::logging::log_step_response;
use crate::extractions::evaluate_step_extractions;
use crate::template::resolve::resolve_template_variables;

#[derive(Debug, Clone)]
pub struct PreparedHttpStep {
    pub step_id: String,
    pub attempt: usize,
    pub max_attempts: usize,
    pub method: Method,
    pub url: Url,
    pub request: StepRequest,
    started_at: Instant,
}

#[derive(Debug)]
pub struct StartedHttpStep {
    pub request: StepRequest,
    pub response: reqwest::Response,
    started_at: Instant,
    attempt: usize,
    max_attempts: usize,
}

pub fn prepare_http_step(
    step: &PipelineStep,
    context: &HashMap<String, StepExecutionResult>,
    specs: Option<&[RuntimeSpec]>,
    env_groups: Option<&[RuntimeEnvGroup]>,
    selected_env_group_slug: Option<&str>,
    attempt: usize,
    max_attempts: usize,
) -> Result<PreparedHttpStep, StepExecutionResult> {
    let started_at = Instant::now();
    let resolved_url = resolve_template_variables(
        &Value::String(step.url.clone()),
        context,
        specs,
        env_groups,
        selected_env_group_slug,
    )
    .as_str()
    .unwrap_or(step.url.as_str())
    .to_owned();

    let resolved_headers = resolve_template_variables(
        &serde_json::to_value(&step.headers).unwrap_or(Value::Object(Map::new())),
        context,
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
        resolve_template_variables(body, context, specs, env_groups, selected_env_group_slug)
    });

    let request = StepRequest {
        method: step.method.clone(),
        url: resolved_url.clone(),
        headers: resolved_headers,
        body: resolved_body,
    };

    let method = match parse_method(&step.method) {
        Ok(method) => method,
        Err(err) => {
            return Err(invalid_step_result(
                step,
                request,
                err,
                started_at,
                attempt,
                max_attempts,
            ));
        }
    };

    let url = match parse_absolute_http_url(&resolved_url) {
        Ok(url) => url,
        Err(err) => {
            return Err(invalid_step_result(
                step,
                request,
                err,
                started_at,
                attempt,
                max_attempts,
            ));
        }
    };

    Ok(PreparedHttpStep {
        step_id: step.id.clone(),
        attempt,
        max_attempts,
        method,
        url,
        request,
        started_at,
    })
}

pub async fn send_prepared_http_step<FCancel>(
    client: &Client,
    prepared: PreparedHttpStep,
    step: &PipelineStep,
    context: &HashMap<String, StepExecutionResult>,
    specs: Option<&[RuntimeSpec]>,
    env_groups: Option<&[RuntimeEnvGroup]>,
    selected_env_group_slug: Option<&str>,
    should_cancel: FCancel,
) -> Option<StepExecutionResult>
where
    FCancel: FnMut() -> bool,
{
    send_prepared_http_step_with_hooks(
        client,
        prepared,
        step,
        context,
        specs,
        env_groups,
        selected_env_group_slug,
        should_cancel,
        || async {},
        || async {},
        || async {},
    )
    .await
}

pub async fn send_prepared_http_step_with_hooks<
    FCancel,
    FStart,
    FStartFuture,
    FSend,
    FSendFuture,
    FBody,
    FBodyFuture,
>(
    client: &Client,
    prepared: PreparedHttpStep,
    step: &PipelineStep,
    context: &HashMap<String, StepExecutionResult>,
    specs: Option<&[RuntimeSpec]>,
    env_groups: Option<&[RuntimeEnvGroup]>,
    selected_env_group_slug: Option<&str>,
    mut should_cancel: FCancel,
    on_send_started: FStart,
    on_send_returned: FSend,
    on_body_completed: FBody,
) -> Option<StepExecutionResult>
where
    FCancel: FnMut() -> bool,
    FStart: FnMut() -> FStartFuture,
    FStartFuture: Future<Output = ()>,
    FSend: FnMut() -> FSendFuture,
    FSendFuture: Future<Output = ()>,
    FBody: FnMut() -> FBodyFuture,
    FBodyFuture: Future<Output = ()>,
{
    let started = start_prepared_http_step_with_hooks(
        client,
        prepared,
        step,
        || should_cancel(),
        on_send_started,
        on_send_returned,
    )
    .await?;

    match started {
        Ok(started) => {
            complete_started_http_step_with_hook(
                started,
                step,
                context,
                specs,
                env_groups,
                selected_env_group_slug,
                should_cancel,
                on_body_completed,
            )
            .await
        }
        Err(result) => Some(result),
    }
}

pub async fn start_prepared_http_step_with_hooks<
    FCancel,
    FStart,
    FStartFuture,
    FSend,
    FSendFuture,
>(
    client: &Client,
    prepared: PreparedHttpStep,
    step: &PipelineStep,
    mut should_cancel: FCancel,
    mut on_send_started: FStart,
    mut on_send_returned: FSend,
) -> Option<Result<StartedHttpStep, StepExecutionResult>>
where
    FCancel: FnMut() -> bool,
    FStart: FnMut() -> FStartFuture,
    FStartFuture: Future<Output = ()>,
    FSend: FnMut() -> FSendFuture,
    FSendFuture: Future<Output = ()>,
{
    let mut request_builder = client.request(prepared.method.clone(), prepared.url.clone());

    for (key, value) in &prepared.request.headers {
        request_builder = request_builder.header(key, value);
    }

    if let Some(body) = prepared.request.body.as_ref() {
        if !prepared.request.method.eq_ignore_ascii_case("GET")
            && !prepared.request.method.eq_ignore_ascii_case("HEAD")
        {
            request_builder = request_builder.json(body);
        }
    }

    let request = prepared.request.clone();
    if should_cancel() {
        return None;
    }
    on_send_started().await;
    let Some(send_result) = await_with_cancel(request_builder.send(), &mut should_cancel).await
    else {
        return None;
    };
    on_send_returned().await;

    match send_result {
        Ok(response) => Some(Ok(StartedHttpStep {
            request,
            response,
            started_at: prepared.started_at,
            attempt: prepared.attempt,
            max_attempts: prepared.max_attempts,
        })),
        Err(err) => {
            let result = step_result(
                step,
                request,
                None,
                Some(err.to_string()),
                "error",
                prepared.started_at,
                prepared.attempt,
                prepared.max_attempts,
                None,
            );
            log_step_response(&step.id, None, result.error.as_deref());
            Some(Err(result))
        }
    }
}

pub async fn complete_started_http_step_with_hook<FCancel, FBody, FBodyFuture>(
    started: StartedHttpStep,
    step: &PipelineStep,
    context: &HashMap<String, StepExecutionResult>,
    specs: Option<&[RuntimeSpec]>,
    env_groups: Option<&[RuntimeEnvGroup]>,
    selected_env_group_slug: Option<&str>,
    mut should_cancel: FCancel,
    mut on_body_completed: FBody,
) -> Option<StepExecutionResult>
where
    FCancel: FnMut() -> bool,
    FBody: FnMut() -> FBodyFuture,
    FBodyFuture: Future<Output = ()>,
{
    let status = started.response.status();
    let status_text = status.canonical_reason().unwrap_or("").to_owned();
    let mut headers = HashMap::new();
    for (key, value) in started.response.headers() {
        headers.insert(
            key.as_str().to_owned(),
            value.to_str().unwrap_or_default().to_owned(),
        );
    }

    let content_type = headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.as_str())
        .unwrap_or("");

    let body = if content_type.contains("application/json") {
        let Some(body_result) =
            await_with_cancel(started.response.json::<Value>(), &mut should_cancel).await
        else {
            return None;
        };
        on_body_completed().await;
        match body_result {
            Ok(value) => value,
            Err(err) => {
                let result = step_result(
                    step,
                    started.request,
                    None,
                    Some(err.to_string()),
                    "error",
                    started.started_at,
                    started.attempt,
                    started.max_attempts,
                    None,
                );
                log_step_response(&step.id, None, result.error.as_deref());
                return Some(result);
            }
        }
    } else {
        let Some(body_result) =
            await_with_cancel(started.response.text(), &mut should_cancel).await
        else {
            return None;
        };
        on_body_completed().await;
        Value::String(body_result.unwrap_or_default())
    };

    let http_error =
        (!status.is_success()).then(|| format!("HTTP {} {}", status.as_u16(), status_text));
    let mut result = step_result(
        step,
        started.request,
        Some(StepResponse {
            status: status.as_u16(),
            status_text: status_text.clone(),
            headers,
            body,
        }),
        http_error.clone(),
        "success",
        started.started_at,
        started.attempt,
        started.max_attempts,
        None,
    );

    let extraction_failed = match evaluate_step_extractions(step, &result) {
        Ok(extracts) => {
            result.extracts = extracts;
            false
        }
        Err(error) => {
            result.status = "error".to_owned();
            result.error = Some(match result.error.take() {
                Some(existing) => format!("{existing} | {error}"),
                None => error,
            });
            true
        }
    };

    if !extraction_failed {
        let has_status_assert = has_status_assertion(step);
        let assert_results = evaluate_assertions(
            step,
            &result,
            context,
            specs,
            env_groups,
            selected_env_group_slug,
        );
        let assertion_failed = assert_results.iter().any(|result| !result.passed);
        if !assert_results.is_empty() {
            if assertion_failed {
                result.status = "error".to_owned();
                let failed_count = assert_results
                    .iter()
                    .filter(|result| !result.passed)
                    .count();
                result.error = Some(match result.error {
                    Some(err) => format!("{} | {} assertion(s) failed", err, failed_count),
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
    }

    log_step_response(&step.id, result.response.as_ref(), result.error.as_deref());
    Some(result)
}

fn invalid_step_result(
    step: &PipelineStep,
    request: StepRequest,
    error: String,
    started_at: Instant,
    attempt: usize,
    max_attempts: usize,
) -> StepExecutionResult {
    step_result(
        step,
        request,
        None,
        Some(error),
        "error",
        started_at,
        attempt,
        max_attempts,
        None,
    )
}

fn step_result(
    step: &PipelineStep,
    request: StepRequest,
    response: Option<StepResponse>,
    error: Option<String>,
    status: &str,
    started_at: Instant,
    attempt: usize,
    max_attempts: usize,
    assert_results: Option<Vec<crate::core::types::AssertionResult>>,
) -> StepExecutionResult {
    StepExecutionResult {
        step_id: step.id.clone(),
        status: status.to_owned(),
        request: Some(request),
        response,
        error,
        duration: Some(started_at.elapsed().as_millis()),
        attempts: if max_attempts > 1 {
            Some(attempt)
        } else {
            None
        },
        attempt: Some(attempt),
        max_attempts: Some(max_attempts),
        extracts: HashMap::new(),
        assert_results,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::{PipelineStep, StepExtraction};
    use httpmock::Method::GET;
    use serde_json::json;
    use std::collections::HashMap;

    #[tokio::test]
    async fn sends_prepared_step_and_returns_success_result() {
        let server = httpmock::MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/users");
                then.status(200)
                    .header("content-type", "application/json")
                    .json_body(json!({"ok": true}));
            })
            .await;

        let client = reqwest::Client::new();
        let step = PipelineStep {
            id: "get-users".to_owned(),
            name: "GET users".to_owned(),
            description: None,
            method: "GET".to_owned(),
            url: format!("{}/users", server.base_url()),
            headers: HashMap::new(),
            body: None,
            operation_id: None,
            delay: None,
            retry: None,
            extracts: Vec::new(),
            asserts: vec![],
        };
        let context = HashMap::new();

        let prepared = prepare_http_step(&step, &context, None, None, None, 1, 1)
            .expect("step should prepare");

        let result =
            send_prepared_http_step(&client, prepared, &step, &context, None, None, None, || {
                false
            })
            .await
            .expect("send should not be cancelled");

        assert_eq!(result.step_id, "get-users");
        assert_eq!(result.status, "success");
        assert_eq!(result.response.as_ref().map(|r| r.status), Some(200));
    }

    #[tokio::test]
    async fn prepared_step_extracts_from_text_response() {
        let server = httpmock::MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/message");
                then.status(200)
                    .header("content-type", "text/html")
                    .body("<strong>123456</strong>");
            })
            .await;
        let step = PipelineStep {
            id: "message".to_owned(),
            name: "Read message".to_owned(),
            description: None,
            method: "GET".to_owned(),
            url: format!("{}/message", server.base_url()),
            headers: HashMap::new(),
            body: None,
            operation_id: None,
            delay: None,
            retry: None,
            asserts: Vec::new(),
            extracts: vec![StepExtraction {
                name: "code".to_owned(),
                field: "body".to_owned(),
                regex: r"<strong>([0-9]{6})</strong>".to_owned(),
                group: 1,
                required: true,
            }],
        };
        let context = HashMap::new();
        let prepared = prepare_http_step(&step, &context, None, None, None, 1, 1)
            .expect("step should prepare");

        let result = send_prepared_http_step(
            &reqwest::Client::new(),
            prepared,
            &step,
            &context,
            None,
            None,
            None,
            || false,
        )
        .await
        .expect("send should not be cancelled");

        assert_eq!(result.extracts.get("code"), Some(&"123456".to_owned()));
    }

    #[tokio::test]
    async fn hooks_report_send_started_before_send_returned() {
        let server = httpmock::MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/users");
                then.status(200)
                    .header("content-type", "application/json")
                    .json_body(json!({"ok": true}));
            })
            .await;

        let client = reqwest::Client::new();
        let step = PipelineStep {
            id: "get-users".to_owned(),
            name: "GET users".to_owned(),
            description: None,
            method: "GET".to_owned(),
            url: format!("{}/users", server.base_url()),
            headers: HashMap::new(),
            body: None,
            operation_id: None,
            delay: None,
            retry: None,
            extracts: Vec::new(),
            asserts: vec![],
        };
        let context = HashMap::new();
        let prepared = prepare_http_step(&step, &context, None, None, None, 1, 1)
            .expect("step should prepare");
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::<&'static str>::new()));

        let started_events = std::sync::Arc::clone(&events);
        let returned_events = std::sync::Arc::clone(&events);
        let result = send_prepared_http_step_with_hooks(
            &client,
            prepared,
            &step,
            &context,
            None,
            None,
            None,
            || false,
            move || {
                let events = std::sync::Arc::clone(&started_events);
                async move {
                    events.lock().expect("events lock").push("started");
                }
            },
            move || {
                let events = std::sync::Arc::clone(&returned_events);
                async move {
                    events.lock().expect("events lock").push("returned");
                }
            },
            || async {},
        )
        .await
        .expect("send should not be cancelled");

        assert_eq!(result.status, "success");
        assert_eq!(
            events.lock().expect("events lock").as_slice(),
            ["started", "returned"]
        );
    }

    #[tokio::test]
    async fn split_http_helpers_start_send_before_body_completion() {
        let server = httpmock::MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/users");
                then.status(200)
                    .header("content-type", "application/json")
                    .json_body(json!({"ok": true}));
            })
            .await;

        let client = reqwest::Client::new();
        let step = PipelineStep {
            id: "get-users".to_owned(),
            name: "GET users".to_owned(),
            description: None,
            method: "GET".to_owned(),
            url: format!("{}/users", server.base_url()),
            headers: HashMap::new(),
            body: None,
            operation_id: None,
            delay: None,
            retry: None,
            extracts: Vec::new(),
            asserts: vec![],
        };
        let context = HashMap::new();
        let prepared = prepare_http_step(&step, &context, None, None, None, 1, 1)
            .expect("step should prepare");
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::<&'static str>::new()));

        let started_events = std::sync::Arc::clone(&events);
        let returned_events = std::sync::Arc::clone(&events);
        let started = start_prepared_http_step_with_hooks(
            &client,
            prepared,
            &step,
            || false,
            move || {
                let events = std::sync::Arc::clone(&started_events);
                async move {
                    events.lock().expect("events lock").push("started");
                }
            },
            move || {
                let events = std::sync::Arc::clone(&returned_events);
                async move {
                    events.lock().expect("events lock").push("returned");
                }
            },
        )
        .await
        .expect("start should not be cancelled")
        .expect("request should start");

        assert_eq!(
            events.lock().expect("events lock").as_slice(),
            ["started", "returned"]
        );

        let body_events = std::sync::Arc::clone(&events);
        let result = complete_started_http_step_with_hook(
            started,
            &step,
            &context,
            None,
            None,
            None,
            || false,
            move || {
                let events = std::sync::Arc::clone(&body_events);
                async move {
                    events.lock().expect("events lock").push("body");
                }
            },
        )
        .await
        .expect("complete should not be cancelled");

        assert_eq!(result.status, "success");
        assert_eq!(
            events.lock().expect("events lock").as_slice(),
            ["started", "returned", "body"]
        );
    }
}
