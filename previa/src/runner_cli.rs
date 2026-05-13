use anyhow::{Context, Result, anyhow, bail};
use reqwest::{Client, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use urlencoding::encode;

use crate::auth::{apply_optional_bearer, auth_path_for_context};
use crate::browser::main_url;
use crate::cli::{RunnerAddArgs, RunnerArgs, RunnerCommands, RunnerListArgs, RunnerSelectorArgs};
use crate::paths::PreviaPaths;
use crate::runtime::read_runtime_state;
use crate::selectors::parse_stack_name;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunnerRecord {
    id: String,
    endpoint: String,
    name: Option<String>,
    source: String,
    enabled: bool,
    health_status: String,
    last_seen_at: Option<String>,
    last_error: Option<String>,
    runtime: Option<Value>,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunnerUpsertBody<'a> {
    endpoint: &'a str,
    name: Option<&'a str>,
    enabled: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunnerUpdateBody<'a> {
    name: Option<&'a str>,
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ApiErrorResponse {
    error: Option<String>,
    message: Option<String>,
}

pub async fn run_runner_cli(paths: &PreviaPaths, http: &Client, args: RunnerArgs) -> Result<()> {
    match args.command {
        RunnerCommands::List(args) => list_runners(paths, http, args).await,
        RunnerCommands::Add(args) => add_runner(paths, http, args).await,
        RunnerCommands::Enable(args) => set_runner_enabled(paths, http, args, true).await,
        RunnerCommands::Disable(args) => set_runner_enabled(paths, http, args, false).await,
        RunnerCommands::Remove(args) => remove_runner(paths, http, args).await,
    }
}

async fn list_runners(paths: &PreviaPaths, http: &Client, args: RunnerListArgs) -> Result<()> {
    let base_url = context_base_url(paths, &args.context)?;
    let auth_path = auth_path_for_context(paths, &args.context)?;
    let runners = fetch_runners(http, &base_url, Some(&auth_path)).await?;

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&runners).context("failed to serialize runners JSON")?
        );
        return Ok(());
    }

    print_runner_rows(&runners);
    Ok(())
}

async fn add_runner(paths: &PreviaPaths, http: &Client, args: RunnerAddArgs) -> Result<()> {
    let base_url = context_base_url(paths, &args.context)?;
    let auth_path = auth_path_for_context(paths, &args.context)?;
    let response =
        apply_optional_bearer(http.post(format!("{base_url}/api/v1/runners")), &auth_path)?
            .json(&RunnerUpsertBody {
                endpoint: &args.endpoint,
                name: args.name.as_deref(),
                enabled: Some(!args.disabled),
            })
            .send()
            .await
            .context("failed to call runner API")?;
    let runner: RunnerRecord = response_json(response, "add runner").await?;
    println!(
        "{} runner '{}' ({})",
        if runner.enabled {
            "registered"
        } else {
            "registered disabled"
        },
        runner.endpoint,
        runner.id
    );
    Ok(())
}

async fn set_runner_enabled(
    paths: &PreviaPaths,
    http: &Client,
    args: RunnerSelectorArgs,
    enabled: bool,
) -> Result<()> {
    let base_url = context_base_url(paths, &args.context)?;
    let auth_path = auth_path_for_context(paths, &args.context)?;
    let runner_id = resolve_runner_id(http, &base_url, Some(&auth_path), &args.selector).await?;
    let response = apply_optional_bearer(
        http.patch(format!("{base_url}/api/v1/runners/{}", encode(&runner_id))),
        &auth_path,
    )?
    .json(&RunnerUpdateBody {
        name: None,
        enabled: Some(enabled),
    })
    .send()
    .await
    .context("failed to call runner API")?;
    let runner: RunnerRecord = response_json(response, "update runner").await?;
    println!(
        "{} runner '{}' ({})",
        if enabled { "enabled" } else { "disabled" },
        runner.endpoint,
        runner.id
    );
    Ok(())
}

async fn remove_runner(paths: &PreviaPaths, http: &Client, args: RunnerSelectorArgs) -> Result<()> {
    let base_url = context_base_url(paths, &args.context)?;
    let auth_path = auth_path_for_context(paths, &args.context)?;
    let runner_id = resolve_runner_id(http, &base_url, Some(&auth_path), &args.selector).await?;
    let response = apply_optional_bearer(
        http.delete(format!("{base_url}/api/v1/runners/{}", encode(&runner_id))),
        &auth_path,
    )?
    .send()
    .await
    .context("failed to call runner API")?;
    ensure_success(response, "remove runner").await?;
    println!("removed runner '{}'", args.selector);
    Ok(())
}

fn context_base_url(paths: &PreviaPaths, context: &str) -> Result<String> {
    let stack_name = parse_stack_name(context)?;
    let stack_paths = paths.stack(&stack_name);
    let state = read_runtime_state(&stack_paths)?.ok_or_else(|| {
        anyhow!(
            "no detached runtime exists for context '{}'",
            stack_paths.name
        )
    })?;
    Ok(main_url(&state.main.address, state.main.port))
}

async fn resolve_runner_id(
    http: &Client,
    base_url: &str,
    auth_path: Option<&std::path::PathBuf>,
    selector: &str,
) -> Result<String> {
    let runners = fetch_runners(http, base_url, auth_path).await?;
    let normalized_endpoint = normalize_endpoint(selector);
    let matches = runners
        .into_iter()
        .filter(|runner| {
            runner.id == selector
                || runner.endpoint == normalized_endpoint
                || runner.name.as_deref() == Some(selector)
        })
        .collect::<Vec<_>>();

    match matches.as_slice() {
        [runner] => Ok(runner.id.clone()),
        [] => bail!("runner '{}' was not found", selector),
        _ => bail!("runner selector '{}' matched multiple runners", selector),
    }
}

async fn fetch_runners(
    http: &Client,
    base_url: &str,
    auth_path: Option<&std::path::PathBuf>,
) -> Result<Vec<RunnerRecord>> {
    let request = http.get(format!("{base_url}/api/v1/runners"));
    let request = match auth_path {
        Some(path) => apply_optional_bearer(request, path)?,
        None => request,
    };
    let response = request.send().await.context("failed to call runner API")?;
    response_json(response, "list runners").await
}

async fn response_json<T: for<'de> Deserialize<'de>>(
    response: Response,
    action: &str,
) -> Result<T> {
    let response = ensure_success(response, action).await?;
    response
        .json::<T>()
        .await
        .with_context(|| format!("{action} returned an invalid response"))
}

async fn ensure_success(response: Response, action: &str) -> Result<Response> {
    if response.status().is_success() {
        return Ok(response);
    }

    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if let Ok(error) = serde_json::from_str::<ApiErrorResponse>(&text) {
        if let Some(message) = error.message {
            if let Some(code) = error.error {
                bail!("{action} failed: HTTP {status}: {code}: {message}");
            }
            bail!("{action} failed: HTTP {status}: {message}");
        }
    }
    bail!("{action} failed: HTTP {status}: {text}");
}

fn normalize_endpoint(value: &str) -> String {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.contains("://") {
        trimmed.to_owned()
    } else {
        format!("http://{trimmed}")
    }
}

fn print_runner_rows(runners: &[RunnerRecord]) {
    if runners.is_empty() {
        println!("no runners registered");
        return;
    }

    println!(
        "{:<36} {:<7} {:<10} {:<8} {:<18} ENDPOINT",
        "ID", "ENABLED", "HEALTH", "SOURCE", "NAME"
    );
    for runner in runners {
        println!(
            "{:<36} {:<7} {:<10} {:<8} {:<18} {}",
            runner.id,
            runner.enabled,
            runner.health_status,
            runner.source,
            runner.name.as_deref().unwrap_or("-"),
            runner.endpoint
        );
    }
}
